use std::io::{Error, ErrorKind, Read, Result, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpListener, TcpStream};

static VERBOSE: usize = 0;

struct Config {
    bind_addr: Vec<SocketAddr>,
}

fn main() {
    let config = Config {
        bind_addr: vec![
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1080),
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 1080),
        ],
    };

    let listener = TcpListener::bind(&config.bind_addr[..])
        .expect(&format!("Can't bind to {:#?}", &config.bind_addr));

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let _ = std::thread::spawn(move || {
                    connection_handshake(&stream)
                        .unwrap_or_else(|e| eprintln!("Connection error: {}", e));
                });
            }
            Err(e) => {
                eprintln!("Connection accept failed {}", e)
            }
        }
    }
}

fn connection_handshake(stream: &TcpStream) -> Result<()> {
    let byte = stream
        .bytes()
        .next()
        .ok_or(error("Connection handshake read failed"))??;
    match byte {
        5 => authentication_negotiation(stream),
        _ => Err(error("Invalid protocol version")),
    }
}

fn authentication_negotiation(mut stream: &TcpStream) -> Result<()> {
    let authentication_methods_count = stream
        .bytes()
        .next()
        .ok_or(error("Authentication method count read failed"))??;
    let mut authentication_methods = vec![0; authentication_methods_count.into()];
    stream.read_exact(&mut authentication_methods)?;

    // Allow unauthenticated user
    if authentication_methods.contains(&0) {
        stream.write(&[5u8, 0u8])?;
        return process_request(stream);
    }

    stream.write(&[5u8, 255u8])?;
    Err(error("No acceptable authentication method sent"))
}

fn process_request(mut stream: &TcpStream) -> Result<()> {
    let mut request = vec![0u8; 4];
    stream.read_exact(&mut request)?;

    // Protocol: always 5
    if request[0] != 5 {
        return Err(error("Request, invalid protocol version"));
    }
    // Command: 1 connect, 3 udp
    if request[1] != 1 && request[1] != 3 {
        stream.write(&[5u8, 7u8])?;
        return Err(error("Request, Invalid command"));
    }
    // Reserved: always 0
    if request[2] != 0 {
        stream.write(&[5u8, 1u8])?;
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
        stream.write(&[5u8, 8u8])?;
        return Err(error("Request, address type not supported"));
    }

    let request_ip = match address_type {
        AddressType::IPv4 => {
            let mut request_addr = vec![0u8; 6];
            stream.read_exact(&mut request_addr)?;
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
            stream.read_exact(&mut request_addr)?;
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
            &stream.peer_addr()?,
            &stream.local_addr()?,
            request_ip
        );
    }

    let remote = TcpStream::connect(request_ip);
    match remote {
        Ok(remote_stream) => {
            let mut local_reader = stream.try_clone()?;
            let mut local_writer = stream.try_clone()?;
            let mut remote_reader = remote_stream.try_clone()?;
            let mut remote_writer = remote_stream.try_clone()?;

            local_writer.write(&[5u8, 0u8, 0u8])?;
            let local_addr = remote_stream.local_addr()?;
            match local_addr.ip() {
                IpAddr::V4(ip) => {
                    local_writer.write(&[1u8])?;
                    local_writer.write(&ip.octets())?;
                }
                IpAddr::V6(ip) => {
                    local_writer.write(&[4u8])?;
                    local_writer.write(&ip.octets())?;
                }
            }
            local_writer.write(&local_addr.port().to_le_bytes())?;

            let sender = std::thread::spawn(move || -> Result<()> {
                let mut buffer = [0u8; 16384];
                loop {
                    match local_reader.read(&mut buffer) {
                        Ok(read) => {
                            if read > 0 {
                                if VERBOSE > 2 {
                                    println!("< {}", &read);
                                }
                                remote_writer.write(&buffer[0..read])?;
                            } else {
                                if VERBOSE > 1 {
                                    println!("Sender EOF");
                                }
                                let _ = remote_writer.shutdown(Shutdown::Both);
                                break;
                            }
                        }
                        Err(e) => {
                            eprintln!("Sender error: {}", e);
                            let _ = remote_writer.shutdown(Shutdown::Both);
                            break;
                        }
                    }
                }
                Ok(())
            });
            let receiver = std::thread::spawn(move || -> Result<()> {
                let mut buffer = [0u8; 16384];
                loop {
                    match remote_reader.read(&mut buffer) {
                        Ok(read) => {
                            if read > 0 {
                                if VERBOSE > 2 {
                                    println!("> {}", &read);
                                }
                                local_writer.write(&buffer[0..read])?;
                            } else {
                                if VERBOSE > 1 {
                                    println!("Receiver EOF");
                                }
                                let _ = local_writer.shutdown(Shutdown::Both);
                                break;
                            }
                        }
                        Err(e) => {
                            eprintln!("Receiver error: {}", e);
                            let _ = local_writer.shutdown(Shutdown::Both);
                            break;
                        }
                    }
                }
                Ok(())
            });
            sender
                .join()
                .expect("The request sender thread has panicked")?;
            receiver
                .join()
                .expect("The request receiver thread has panicked")?;
        }
        Err(_) => {
            stream.write(&[5u8, 5u8])?;
            return Err(error("Request, connection failed"));
        }
    }
    Ok(())
}

fn error(e: &'static str) -> Error {
    Error::new(ErrorKind::Other, e)
}
