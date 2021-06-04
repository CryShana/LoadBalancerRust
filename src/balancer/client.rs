use std::io::prelude::*;
use std::io::ErrorKind;
use std::io::Result;
use std::net::Shutdown;
use std::net::SocketAddr;
use std::time::Duration;
use std::time::Instant;

use mio::net::TcpStream;
use socket2::{Domain, Socket, Type};

pub struct TcpClient {
    stream: TcpStream,
    buffer: [u8; 4096],

    pub address: SocketAddr,
    target: Option<SocketAddr>,
    target_stream: Option<Socket>,
    is_connected: bool,
    is_connecting: bool,
    is_client_connected: bool,
    already_connected_code: i32,
    last_connection_loss: Instant,
    connection_started_time: Instant,

    last_target: Option<SocketAddr>,
    last_target_error: bool,
}

impl TcpClient {
    pub fn new(stream: TcpStream) -> Self {

        let addr: SocketAddr = stream.peer_addr().unwrap();
        println!("[Listener] Connected from {}", addr.to_string());

        // determine OS error code for "already connected socket"
        let code;
        let os = std::env::consts::OS;
        if os == "windows" {
            code = 10056;
        } else {
            // linux
            code = 106;
        }

        TcpClient {
            stream: stream,
            buffer: [0; 4096],
            target: None,
            target_stream: None,
            address: addr,
            is_connected: false,
            is_connecting: false,
            is_client_connected: true,
            already_connected_code: code,
            last_connection_loss: Instant::now(),
            connection_started_time: Instant::now(),
            last_target: None,
            last_target_error: false,
        }
    }

    pub fn get_target_addr(&self) -> Option<SocketAddr> {
        self.target
    }

    pub fn get_last_target_addr(&self) -> Option<SocketAddr> {
        self.last_target
    }

    pub fn last_target_errored(&self) -> bool {
        self.last_target_error
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected
    }

    pub fn is_connecting(&self) -> bool {
        self.is_connecting
    }

    pub fn is_client_connected(&self) -> bool {
        self.is_client_connected
    }

    pub fn connect_to_target(&mut self, target: SocketAddr, timeout: Duration, total_timeout: Duration) -> Result<bool> {
        if !self.is_connecting || self.target_stream.is_none() {
            self.close_connection_to_target(false);

            // prepare for new connection - initialize socket and set target
            let mut domain = Domain::IPV4;
            if target.is_ipv6() {
                domain = Domain::IPV6;
            }

            let socket = Socket::new(domain, Type::STREAM, None)?;
            socket.set_nonblocking(true)?;

            self.is_connecting = true;
            self.target = Some(target);
            self.target_stream = Some(socket);
            self.connection_started_time = Instant::now() + timeout;
        }

        // use the previously initialized target and socket (target parameter is ignored when client is connecting)
        let socket = self.target_stream.as_ref().unwrap();
        let target = self.target.unwrap();

        // check if we timed out for this target
        if Instant::now() > self.connection_started_time {
            self.close_connection_to_target(true);
            return Ok(false);
        }

        if Instant::now() > self.last_connection_loss + total_timeout {
            self.close_connection();
            return Ok(false);
        }

        // initiate connection here
        match socket.connect(&target.into()) {
            Ok(()) => {
                self.set_connected();
                return Ok(true);
            }
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                return Ok(false);
            }
            Err(ref e) if e.kind() == ErrorKind::Other => {
                let code = e.raw_os_error().unwrap_or(0);
                if code != self.already_connected_code {
                    // actual error
                    return Ok(false);
                }

                self.set_connected();
                return Ok(true);
            }
            Err(_) => {
                self.close_connection_to_target(true);
                return Ok(false);
            }
        };
    }

    fn set_connected(&mut self) {
        self.is_connected = true;
        self.is_connecting = false;
    }

    /**
        Reads from client and forwards it to server. First boolean represents processing success, will be [false] when connection to either client or server fails.
        Second boolean represents if any processing has actually been done, if no data has been read or written, [false] will be returned.
    */
    pub fn process(&mut self) -> (bool, bool) {
        let mut str = self.target_stream.as_ref().unwrap();
        let mut has_processed = false;

        // READ FROM CLIENT
        let read: i32 = match self.stream.read(&mut self.buffer) {
            Ok(r) => r as i32,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => -1,
            Err(_) => {
                // error with connection to client
                self.close_connection();
                return (false, has_processed);
            }
        };

        // WRITE TO SERVER
        if read > 0 {
            match str.write(&self.buffer[..(read as usize)]) {
                Ok(_) => {
                    has_processed = true;
                }
                Err(_) => {
                    // error with connection to server
                    self.close_connection_to_target(true);
                    return (false, has_processed);
                }
            }
        } else if read == 0 {
            self.close_connection();
            return (false, has_processed);
        }

        // READ FROM SERVER
        let reads: i32 = match str.read(&mut self.buffer) {
            Ok(r) => r as i32,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => -1,
            Err(_) => {
                // error with connection to server
                self.close_connection_to_target(true);
                return (false, has_processed);
            }
        };

        // WRITE TO CLIENT
        if reads > 0 {
            match self.stream.write(&self.buffer[..(reads as usize)]) {
                Ok(_) => {
                    has_processed = true;
                }
                Err(_) => {
                    // error with connection to client
                    self.close_connection();
                    return (false, has_processed);
                }
            };
        } else if reads == 0 {
            self.close_connection_to_target(false);
            return (false, has_processed);
        }

        return (true, has_processed);
    }

    fn close_connection_to_target(&mut self, target_errored: bool) {
        // if connected to target, disconnect - mark last connection loss
        if self.is_connected {
            let str = self.target_stream.as_ref().unwrap();
            str.shutdown(Shutdown::Both).unwrap_or(());
            drop(str);

            self.last_connection_loss = Instant::now();
        }

        // mark error
        if target_errored {
            self.last_target = self.target;
            self.last_target_error = true;
        } else {
            self.last_target = None;
            self.last_target_error = false;
        }

        // reset
        self.target = None;
        self.target_stream = None;

        self.is_connected = false;
        self.is_connecting = false;
    }

    fn close_connection(&mut self) {
        if self.is_client_connected {
            let str = &self.stream;
            str.shutdown(Shutdown::Both).unwrap_or(());
            drop(str);
            
            self.is_client_connected = false;

            // also close connection to target if connected - there is no reason to stay connected if client is not
            self.close_connection_to_target(false);
        }
    }
}

impl Drop for TcpClient {
    fn drop(&mut self) {
        self.close_connection();
    }
}
