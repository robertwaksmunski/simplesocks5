use std::io::{Error, ErrorKind, Read, Result, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpListener, TcpStream};

static VERBOSE: usize = 0;

fn main() {
    let listener = TcpListener::bind("0.0.0.0:1080").expect("Can't bind to local port");

    for client_stream in listener.incoming() {
        match client_stream {
            Ok(client_stream) => {
                let _ = client_stream.set_nodelay(true);
                let _ = std::thread::spawn(move || {
                    connection_handshake(&client_stream)
                        .unwrap_or_else(|e| eprintln!("Connection error: {}", e));
                });
            }
            Err(e) => {
                eprintln!("Connection accept failed {}", e)
            }
        }
    }
}

fn connection_handshake(client_stream: &TcpStream) -> Result<()> {
    let byte = client_stream
        .bytes()
        .next()
        .ok_or(error("Connection handshake read failed"))??;
    match byte {
        5 => authentication_negotiation(client_stream),
        _ => Err(error("Invalid protocol version")),
    }
}

fn authentication_negotiation(mut client_stream: &TcpStream) -> Result<()> {
    let authentication_methods_count = client_stream
        .bytes()
        .next()
        .ok_or(error("Authentication method count read failed"))??;
    let mut authentication_methods = vec![0; authentication_methods_count.into()];
    client_stream.read_exact(&mut authentication_methods)?;

    // Allow unauthenticated user
    if authentication_methods.contains(&0) {
        client_stream.write(&[5u8, 0u8])?;
        return process_request(client_stream);
    }

    client_stream.write(&[5u8, 255u8])?;
    Err(error("No acceptable authentication method sent"))
}

fn process_request(mut client_stream: &TcpStream) -> Result<()> {
    let mut request = vec![0u8; 4];
    client_stream.read_exact(&mut request)?;

    // Protocol: always 5
    if request[0] != 5 {
        return Err(error("Request, invalid protocol version"));
    }
    // Command: 1 connect, 3 udp
    if request[1] != 1 && request[1] != 3 {
        client_stream.write(&[5u8, 7u8])?;
        return Err(error("Request, Invalid command"));
    }
    // Reserved: always 0
    if request[2] != 0 {
        client_stream.write(&[5u8, 1u8])?;
        return Err(error("Request, Invalid reserved"));
    }
    // Address type: 1 IPv4, 4 IPv6
    enum AddressType {
        IPv4,
        IPv6,
    }
    let address_type;
    if request[3] == 4 {
        address_type = AddressType::IPv6;
    } else if request[3] == 1 {
        address_type = AddressType::IPv4;
    } else {
        client_stream.write(&[5u8, 8u8])?;
        return Err(error("Request, address type not supported"));
    }

    let request_ip = match address_type {
        AddressType::IPv4 => {
            let mut request_addr = vec![0u8; 6];
            client_stream.read_exact(&mut request_addr)?;
            SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(
                    request_addr[0],
                    request_addr[1],
                    request_addr[2],
                    request_addr[3],
                )),
                u16::from_be_bytes([request_addr[4], request_addr[5]]),
            )
        }
        AddressType::IPv6 => {
            let mut request_addr = vec![0u8; 18];
            client_stream.read_exact(&mut request_addr)?;
            SocketAddr::new(
                IpAddr::V6(Ipv6Addr::new(
                    u16::from_be_bytes([request_addr[0], request_addr[1]]),
                    u16::from_be_bytes([request_addr[2], request_addr[3]]),
                    u16::from_be_bytes([request_addr[4], request_addr[5]]),
                    u16::from_be_bytes([request_addr[6], request_addr[7]]),
                    u16::from_be_bytes([request_addr[8], request_addr[9]]),
                    u16::from_be_bytes([request_addr[10], request_addr[11]]),
                    u16::from_be_bytes([request_addr[12], request_addr[13]]),
                    u16::from_be_bytes([request_addr[14], request_addr[15]]),
                )),
                u16::from_be_bytes([request_addr[16], request_addr[17]]),
            )
        }
    };
    if VERBOSE > 0 {
        println!(
            "Client {} connected to {} requests {}",
            &client_stream.peer_addr()?,
            &client_stream.local_addr()?,
            request_ip
        );
    }

    let remote = TcpStream::connect(request_ip);
    match remote {
        Ok(mut remote_stream) => {
            let _ = remote_stream.set_nodelay(true);

            client_stream.write(&[5u8, 0u8, 0u8])?;
            let local_addr = remote_stream.local_addr()?;
            match local_addr.ip() {
                IpAddr::V4(ip) => {
                    client_stream.write(&[1u8])?;
                    client_stream.write(&ip.octets())?;
                }
                IpAddr::V6(ip) => {
                    client_stream.write(&[4u8])?;
                    client_stream.write(&ip.octets())?;
                }
            }
            client_stream.write(&local_addr.port().to_le_bytes())?;

            let mut client_stream_clone = client_stream.try_clone()?;
            let mut remote_stream_clone = remote_stream.try_clone()?;

            let receiver = std::thread::spawn(move || -> Result<()> {
                pipe_data(
                    "Recv",
                    &mut remote_stream_clone,
                    &mut client_stream_clone,
                )
            });

            pipe_data(
                "Send",
                &mut client_stream.try_clone()?,
                &mut remote_stream,
            )?;

            receiver
                .join()
                .expect("The request receiver thread has panicked")?;
        }
        Err(_) => {
            client_stream.write(&[5u8, 5u8])?;
            return Err(error("Request, connection failed"));
        }
    }
    Ok(())
}

fn pipe_data(name: &str, from: &mut TcpStream, to: &mut TcpStream) -> Result<()> {
    let mut buffer = [0u8; 16384];
    loop {
        match from.read(&mut buffer) {
            Ok(read) => {
                if read > 0 {
                    if VERBOSE > 2 {
                        println!("{}: {}", name, &read);
                    }
                    to.write(&buffer[0..read])?;
                } else {
                    if VERBOSE > 1 {
                        println!("{}: EOF", name);
                    }
                    let _ = to.shutdown(Shutdown::Both);
                    break;
                }
            }
            Err(e) => {
                eprintln!("{} error: {}", name, e);
                let _ = to.shutdown(Shutdown::Both);
                break;
            }
        }
    }
    Ok(())
}

fn error(e: &'static str) -> Error {
    Error::new(ErrorKind::Other, e)
}
