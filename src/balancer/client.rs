use std::io::prelude::*;
use std::net::Shutdown;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::time::Duration;

pub struct TcpClient {
    stream: TcpStream,
    buffer: [u8; 4096],

    target: Option<SocketAddr>,
    target_stream: Option<TcpStream>,
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

    pub fn is_connected(&self) -> bool {
        self.is_connected
    }

    pub fn is_connecting(&self) -> bool {
        self.is_connecting
    }

    pub fn is_client_connected(&self) -> bool {
        self.is_client_connected
    }

    pub fn connect_to_target(&mut self, target: SocketAddr, timeout: Duration) -> bool {
        self.close_connection_to_target();
        self.is_connecting = true;

        let str = match TcpStream::connect_timeout(&target, timeout) {
            Ok(stream) => stream,
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                // timed out
                return false;
            }
            Err(err) => {
                // error while trying to connect (msg: err.to_string())
                /*
                println!(
                    "[{} <-> {}] Error while trying to connect to server: {}",
                    self.address,
                    target,
                    err.to_string()
                );*/
                return false;
            }
        };

        str.set_nonblocking(true).unwrap();

        self.target = Some(target);
        self.target_stream = Some(str);
        self.is_connected = true;
        self.is_connecting = false;

        return true;
    }

    /**
        Reads from client and forwards it to server. Returns [false] when connection to either client or server fails.
    */
    pub fn process(&mut self) -> bool {
        // do not process if target host is not set/connected
        let target = match self.target {
            Some(t) => t,
            None => return true,
        };

        let mut str = self.target_stream.as_ref().unwrap();

        // READ FROM CLIENT
        let read: i32 = match self.stream.read(&mut self.buffer) {
            Ok(r) => r as i32,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => -1,
            Err(err) => {
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
            Err(err) => {
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
