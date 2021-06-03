use std::io::prelude::*;
use std::io::ErrorKind;
use std::io::Result;
use std::net::Shutdown;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::time::Duration;

use socket2::{Domain, Socket, Type};

pub struct TcpClient {
    stream: TcpStream,
    buffer: [u8; 4096],

    target: Option<SocketAddr>,
    target_stream: Option<Socket>,
    is_connected: bool,
    is_connecting: bool,
    is_client_connected: bool,
    pub address: SocketAddr,
}

impl TcpClient {
    pub fn new(stream: TcpStream) -> Self {
        stream.set_nonblocking(true).unwrap();

        let addr = stream.peer_addr().unwrap();
        println!("[Listener] Connected from {}", addr.to_string());

        TcpClient {
            stream: stream,
            buffer: [0; 4096],
            target: None,
            target_stream: None,
            address: addr,
            is_connected: false,
            is_connecting: false,
            is_client_connected: true,
        }
    }

    pub fn get_target_addr(&self) -> Option<SocketAddr> {
        self.target
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

    pub fn connect_to_target(&mut self, target: SocketAddr, timeout: Duration) -> Result<bool> {
        if !self.is_connecting || self.target_stream.is_none() {
            self.close_connection_to_target();

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
        }

        // TODO: maybe use SO_KEEPALIVE socket option to check for dead connections?

        // use the previously initialized target and socket (target parameter is ignored when client is connecting)
        let socket = self.target_stream.as_ref().unwrap();
        let target = self.target.unwrap();

        // initiate connection here
        match socket.connect(&target.into()) {
            Ok(()) => {
                self.is_connected = true;
                self.is_connecting = false;
                return Ok(true);
            }
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                return Ok(false);
            }
            Err(ref e) if e.kind() == ErrorKind::Other => {
                // WINDOWS 10: If socket already connected, then os error = 10056
                let code = e.raw_os_error().unwrap_or(0);

                self.is_connected = true;
                self.is_connecting = false;
                
                println!("Connected - {} - Code: {}", e.to_string(), code);
                return Ok(true);
            }
            Err(err) => {
                //self.is_connecting = false;
                println!("Error? {} -- KIND: {:?}", err.to_string(), err.kind());
                return Ok(false);
            }
        };
    }

    /**
        Reads from client and forwards it to server. Returns [false] when connection to either client or server fails.
    */
    pub fn process(&mut self) -> bool {
        let mut str = self.target_stream.as_ref().unwrap();

        // READ FROM CLIENT
        let read: i32 = match self.stream.read(&mut self.buffer) {
            Ok(r) => r as i32,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => -1,
            Err(_) => {
                // error with connection to client
                self.close_connection();
                return false;
            }
        };

        // WRITE TO SERVER
        if read > 0 {
            str.write(&self.buffer[..(read as usize)]).unwrap();
        } else if read == 0 {
            //println!("[{} <-> {}] Zero buffer from client", self.address, target);
            self.close_connection();
            return false;
        }

        // READ FROM SERVER
        let reads: i32 = match str.read(&mut self.buffer) {
            Ok(r) => r as i32,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => -1,
            Err(_) => {
                // error with connection to server
                self.close_connection_to_target();
                return false;
            }
        };

        // WRITE TO CLIENT
        if reads > 0 {
            self.stream.write(&self.buffer[..(reads as usize)]).unwrap();
        } else if reads == 0 {
            //println!("[{} <-> {}] Zero buffer from server", self.address, target);
            self.close_connection_to_target();
            return false;
        }

        return true;
    }

    fn close_connection_to_target(&mut self) {
        if self.is_connected {
            let str = self.target_stream.as_ref().unwrap();
            str.shutdown(Shutdown::Both).expect("Failed to shutdown server TCP stream");

            self.target = None;
            self.target_stream = None;

            self.is_connected = false;
        }
    }

    fn close_connection(&mut self) {
        if self.is_client_connected {
            self.stream.shutdown(Shutdown::Both).expect("Failed to shutdown client TCP stream");

            self.is_client_connected = false;

            // also close connection to target if connected - there is no reason to stay connected if client is not
            self.close_connection_to_target();
        }
    }
}

impl Drop for TcpClient {
    fn drop(&mut self) {
        self.close_connection();
    }
}
