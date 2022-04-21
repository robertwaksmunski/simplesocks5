use std::env;
use std::io::{Error, ErrorKind, Read, Result, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::str::FromStr;
use std::thread;
// Look, no external dependencies, pure rust, pure std lib.

fn main() {
    // Parse agruments
    let verbosity_level = env::args()
        .into_iter()
        .find(|s| s.starts_with("-v"))
        .unwrap_or_else(|| String::from("-"))
        .len()
        - 1;

    let bind = match env::args()
        .into_iter()
        .find(|s| SocketAddr::from_str(s).is_ok())
    {
        Some(s) => SocketAddr::from_str(&s),
        None => SocketAddr::from_str("0.0.0.0:1080"),
    }
    .expect("Valid bind address needed");

    println!(
        "Listening on {} with verbosity level {}",
        bind, verbosity_level
    );

    // Bind and listen for connections
    let listener = TcpListener::bind(bind).expect("Can't bind to local port");

    for client_stream in listener.incoming() {
        match client_stream {
            Ok(client_stream) => {
                let _ = client_stream.set_nodelay(true);
                let _ = thread::spawn(move || {
                    handle_connection(&client_stream, verbosity_level).unwrap_or_else(|e| {
                        eprintln!("Connection {:?} error: {}", &client_stream, e)
                    });
                });
            }
            Err(e) => {
                eprintln!("Connection accept failed {}", e)
            }
        }
    }
}

fn handle_connection(client_stream: &TcpStream, verbosity_level: usize) -> Result<()> {
    connection_handshake(client_stream)?;
    authentication_negotiation(client_stream)?;
    if verbosity_level > 0 {
        println!("Connection {:?} successful", &client_stream);
    }
    let remote_ip = parse_request(client_stream, verbosity_level)?;
    remote_request(&remote_ip, client_stream, verbosity_level)
}

fn connection_handshake(client_stream: &TcpStream) -> Result<()> {
    let byte = client_stream
        .bytes()
        .next()
        .ok_or_else(|| error("Connection handshake read failed"))??;
    match byte {
        5 => Ok(()),
        _ => Err(error("Invalid protocol version")),
    }
}

fn authentication_negotiation(mut client_stream: &TcpStream) -> Result<()> {
    let authentication_methods_count = client_stream
        .bytes()
        .next()
        .ok_or_else(|| error("Authentication method count read failed"))??;
    let mut authentication_methods = vec![0; authentication_methods_count.into()];
    client_stream.read_exact(&mut authentication_methods)?;

    // Allow unauthenticated user
    if authentication_methods.contains(&0) {
        client_stream.write_all(&[5u8, 0u8])?;
        return Ok(());
    }

    client_stream.write_all(&[5u8, 255u8])?;
    Err(error("No acceptable authentication method sent"))
}

fn parse_request(mut client_stream: &TcpStream, verbosity_level: usize) -> Result<SocketAddr> {
    enum AddressType {
        IPv4,
        IPv6,
    }

    let mut request = [0u8; 4];
    client_stream.read_exact(&mut request)?;

    // Protocol: always 5
    if request[0] != 5 {
        return Err(error("Request, invalid protocol version"));
    }
    // Command: 1 connect, 3 udp
    if request[1] != 1 {
        client_stream.write_all(&[5u8, 7u8])?;
        return Err(error("Request, Invalid command"));
    }
    // Reserved: always 0
    if request[2] != 0 {
        client_stream.write_all(&[5u8, 1u8])?;
        return Err(error("Request, Invalid reserved"));
    }
    // Address type: 1 IPv4, 4 IPv6
    let address_type;
    if request[3] == 4 {
        address_type = AddressType::IPv6;
    } else if request[3] == 1 {
        address_type = AddressType::IPv4;
    } else {
        client_stream.write_all(&[5u8, 8u8])?;
        return Err(error("Request, address type not supported"));
    }

    let request_ip = match address_type {
        AddressType::IPv4 => {
            let mut request_addr = [0u8; 6];
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
            let mut request_addr = [0u8; 18];
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
    if verbosity_level > 0 {
        println!("Connection {:?} requests {}", &client_stream, request_ip);
    }
    Ok(request_ip)
}

fn remote_request(
    request_ip: &SocketAddr,
    mut client_stream: &TcpStream,
    verbosity_level: usize,
) -> Result<()> {
    let remote = TcpStream::connect(request_ip);
    match remote {
        Ok(mut remote_stream) => {
            let _ = remote_stream.set_nodelay(true);

            client_stream.write_all(&[5u8, 0u8, 0u8])?;
            let local_addr = remote_stream.local_addr()?;
            match local_addr.ip() {
                IpAddr::V4(ip) => {
                    client_stream.write_all(&[1u8])?;
                    client_stream.write_all(&ip.octets())?;
                }
                IpAddr::V6(ip) => {
                    client_stream.write_all(&[4u8])?;
                    client_stream.write_all(&ip.octets())?;
                }
            }
            client_stream.write_all(&local_addr.port().to_le_bytes())?;

            let mut client_stream_clone = client_stream.try_clone()?;
            let mut remote_stream_clone = remote_stream.try_clone()?;

            let receiver = thread::spawn(move || -> Result<()> {
                pipe_data(
                    "Recv",
                    &mut remote_stream_clone,
                    &mut client_stream_clone,
                    verbosity_level,
                )
            });

            pipe_data(
                "Send",
                &mut client_stream.try_clone()?,
                &mut remote_stream,
                verbosity_level,
            )?;

            receiver
                .join()
                .expect("The request receiver thread has panicked")?;
        }
        Err(_) => {
            client_stream.write_all(&[5u8, 5u8])?;
            return Err(error("Request, connection failed"));
        }
    }
    Ok(())
}

fn pipe_data(
    name: &str,
    from: &mut TcpStream,
    to: &mut TcpStream,
    verbosity_level: usize,
) -> Result<()> {
    let mut buffer = [0u8; 16384];
    loop {
        match from.read(&mut buffer) {
            Ok(read) => {
                if read > 0 {
                    if verbosity_level > 2 {
                        println!("{}: {}", name, &read);
                    }
                    to.write_all(&buffer[0..read])?;
                } else {
                    if verbosity_level > 1 {
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
