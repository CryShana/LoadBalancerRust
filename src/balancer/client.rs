use std::io::prelude::*;
use std::io::ErrorKind;
use std::io::Result;
use std::net::Shutdown;
use std::net::SocketAddr;

use std::time::Duration;
use std::time::Instant;

use mio::net::TcpStream;
use mio::Interest;
use mio::Poll;
use mio::Token;

pub struct TcpClient {
    pub stream: TcpStream,
    buffer: [u8; 4096],

    pub address: SocketAddr,
    target: Option<SocketAddr>,
    target_stream: Option<TcpStream>,
    is_connected: bool,
    is_connecting: bool,
    is_client_connected: bool,
    pub last_connection_loss: Instant,
    pub started_connecting: Instant,
    last_target: Option<SocketAddr>,
    last_target_error: bool,
}

impl TcpClient {
    pub fn new(stream: TcpStream) -> Self {
        let addr: SocketAddr = stream.peer_addr().unwrap();
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
            last_connection_loss: Instant::now(),
            started_connecting: Instant::now(),
            last_target: None,
            last_target_error: false,
        }
    }

    pub fn register_target_with_poll(&mut self, poll: &Poll, token: Token) -> Option<()> {
        let mut str = self.target_stream.take()?;

        poll.registry().register(&mut str, token, Interest::READABLE | Interest::WRITABLE).unwrap();

        self.target_stream = Some(str);

        Some(())
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

    pub fn connect_to_target(&mut self, target: SocketAddr) -> Result<bool> {
        if self.is_connecting {
            println!("[WARNING] Already connecting, this shouldn't happen");
            return Ok(false);
        }

        self.close_connection_to_target(false);

        // start connecting
        let stream = match TcpStream::connect(target) {
            Ok(t) => t,
            Err(_) => {
                return Ok(false);
            }
        };

        self.is_connecting = true;
        self.target = Some(target);
        self.target_stream = Some(stream);
        self.started_connecting = Instant::now();

        Ok(true)
    }

    pub fn check_target_connected(&mut self) -> Result<bool> {
        let stream = self.target_stream.as_ref().unwrap();

        let mut buf: [u8; 1] = [0; 1];
        match stream.peek(&mut buf) {
            Ok(_) => true,
            Err(ref e) if e.kind() == ErrorKind::NotConnected => return Ok(false),
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => true,
            Err(e) => {
                return Err(e);
            }
        };

        self.set_connected();
        Ok(true)
    }

    fn set_connected(&mut self) {
        self.is_connected = true;
        self.is_connecting = false;
    }

    /**
        Reads from client and forwards it to server. Boolean represents processing success, will be [false] when connection to either client or server fails.
        Equivalent of calling [forward_to_target] and [forward_from_target] methods
    */
    pub fn process(&mut self) -> bool {
        if !self.forward_to_target() {
            return false;
        }

        if !self.forward_from_target() {
            return false;
        }

        return true;
    }

    /**
        Forwards client messages to connected target. (Reads from client stream and writes to target stream)
    */
    pub fn forward_to_target(&mut self) -> bool {
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
            match str.write(&self.buffer[..(read as usize)]) {
                Ok(_) => {}
                Err(_e) => {
                    // error with connection to server
                    self.close_connection_to_target(true);
                    return false;
                }
            }
        } else if read == 0 {
            self.close_connection();
            return false;
        }

        return true;
    }

     /**
        Forwards connected target messages to client. (Reads from target stream and writes to client stream)
    */
    pub fn forward_from_target(&mut self) -> bool {
        let mut str = self.target_stream.as_ref().unwrap();

        // READ FROM SERVER
        let reads: i32 = match str.read(&mut self.buffer) {
            Ok(r) => r as i32,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => -1,
            Err(_e) => {
                // error with connection to server
                self.close_connection_to_target(true);
                return false;
            }
        };

        // WRITE TO CLIENT
        if reads > 0 {
            match self.stream.write(&self.buffer[..(reads as usize)]) {
                Ok(_) => {}
                Err(_) => {
                    // error with connection to client
                    self.close_connection();
                    return false;
                }
            };
        } else if reads == 0 {
            self.close_connection_to_target(false);
            return false;
        }

        return true;
    }

    pub fn close_connection_to_target(&mut self, target_errored: bool) {
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

    pub fn close_connection(&mut self) {
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
