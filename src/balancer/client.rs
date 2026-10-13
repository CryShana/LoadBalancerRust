use std::io::{ErrorKind, Read, Result, Write};
use std::net::{Shutdown, SocketAddr};
use std::time::Instant;

use mio::net::TcpStream;
use mio::{Interest, Registry, Token};

/**
    Client connection paired with the target host it is being forwarded to
*/
pub struct TcpClient {
    pub address: SocketAddr,
    pub accepted_at: Instant,
    pub last_activity: Instant,
    token: Token,
    stream: TcpStream,
    interest: Option<Interest>,
    target: Option<Target>,
    to_target: Pipe,
    to_client: Pipe,
}

struct Target {
    address: SocketAddr,
    stream: TcpStream,
    interest: Option<Interest>,
    connected: bool,
    started_at: Instant,
}

/**
    One direction of the forwarded connection. Holds bytes that were read but not yet accepted by the receiving socket
*/
#[derive(Default)]
struct Pipe {
    pending: Vec<u8>,
    closed: bool,
}

impl TcpClient {
    pub fn new(mut stream: TcpStream, token: Token, registry: &Registry) -> Result<Self> {
        let address = stream.peer_addr()?;
        stream.set_nodelay(true)?;
        registry.register(&mut stream, token, Interest::READABLE)?;

        Ok(TcpClient {
            address,
            accepted_at: Instant::now(),
            last_activity: Instant::now(),
            token,
            stream,
            interest: Some(Interest::READABLE),
            target: None,
            to_target: Pipe::default(),
            to_client: Pipe::default(),
        })
    }

    pub fn target_addr(&self) -> Option<SocketAddr> {
        self.target.as_ref().map(|t| t.address)
    }

    pub fn target_started_at(&self) -> Option<Instant> {
        self.target.as_ref().map(|t| t.started_at)
    }

    pub fn is_connected(&self) -> bool {
        self.target.as_ref().is_some_and(|t| t.connected)
    }

    /**
        Starts a non-blocking connection to the target, dropping any previous target connection
    */
    pub fn connect(&mut self, address: SocketAddr, registry: &Registry) -> Result<()> {
        self.disconnect_target(registry);

        let mut stream = TcpStream::connect(address)?;
        // writable readiness signals that the connection attempt has finished
        let interest = Interest::READABLE | Interest::WRITABLE;
        registry.register(&mut stream, self.token, interest)?;

        self.target = Some(Target {
            address,
            stream,
            interest: Some(interest),
            connected: false,
            started_at: Instant::now(),
        });

        Ok(())
    }

    /**
        Returns whether the pending target connection has been established. An error means the connection attempt failed
    */
    pub fn check_connected(&mut self) -> Result<bool> {
        let Some(target) = self.target.as_mut() else {
            return Ok(false);
        };

        if let Some(e) = target.stream.take_error()? {
            return Err(e);
        }

        match target.stream.peer_addr() {
            Ok(_) => {
                target.stream.set_nodelay(true)?;
                target.connected = true;
                Ok(true)
            }
            Err(e) if e.kind() == ErrorKind::NotConnected => Ok(false),
            Err(e) => Err(e),
        }
    }

    /**
        Forwards data in both directions until the sockets would block. Returns false once both sides have closed
    */
    pub fn forward(&mut self, buffer: &mut [u8], registry: &Registry) -> Result<bool> {
        let target = self.target.as_mut().expect("forward requires a connected target");
        self.last_activity = Instant::now();

        // CLIENT -> TARGET
        pump(&mut self.stream, &mut target.stream, &mut self.to_target, buffer)?;

        // TARGET -> CLIENT
        pump(&mut target.stream, &mut self.stream, &mut self.to_client, buffer)?;

        // UPDATE INTERESTS (only wait for readiness that can make progress)
        let client_interest = wanted_interest(&self.to_target, &self.to_client);
        set_interest(registry, &mut self.stream, self.token, &mut self.interest, client_interest)?;

        let target_interest = wanted_interest(&self.to_client, &self.to_target);
        set_interest(registry, &mut target.stream, self.token, &mut target.interest, target_interest)?;

        Ok(!(self.to_target.closed && self.to_client.closed))
    }

    fn disconnect_target(&mut self, registry: &Registry) {
        if let Some(mut target) = self.target.take() {
            let _ = registry.deregister(&mut target.stream);
        }
    }

    pub fn close(&mut self, registry: &Registry) {
        self.disconnect_target(registry);
        let _ = registry.deregister(&mut self.stream);
    }
}

/**
    Moves data from one socket to the other until either would block. Mio is edge-triggered,
    so the source has to be read until it would block or no further readiness event arrives
*/
fn pump(from: &mut TcpStream, to: &mut TcpStream, pipe: &mut Pipe, buffer: &mut [u8]) -> Result<()> {
    // WRITE PENDING DATA (stop reading until receiver accepts it)
    while !pipe.pending.is_empty() {
        match write_some(to, &pipe.pending)? {
            Some(n) => drop(pipe.pending.drain(..n)),
            None => return Ok(()),
        }
    }

    while !pipe.closed {
        // READ MORE DATA
        let read = match from.read(buffer) {
            Ok(0) => {
                // propagate the half-close so the other side still gets to finish sending its data
                let _ = to.shutdown(Shutdown::Write);
                pipe.closed = true;
                break;
            }
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };

        // WRITE IT THROUGH (keep whatever the receiver doesn't accept yet)
        let mut written = 0;
        while written < read {
            match write_some(to, &buffer[written..read])? {
                Some(n) => written += n,
                None => {
                    pipe.pending.extend_from_slice(&buffer[written..read]);
                    return Ok(());
                }
            }
        }
    }

    Ok(())
}

/**
    Socket readiness worth waiting for. Writable interest is only registered while data is pending,
    otherwise mio on Windows keeps reporting the always-writable socket and the worker busy-loops
*/
fn wanted_interest(reading: &Pipe, writing: &Pipe) -> Option<Interest> {
    let readable = (!reading.closed).then_some(Interest::READABLE);
    let writable = (!writing.pending.is_empty()).then_some(Interest::WRITABLE);

    match (readable, writable) {
        (Some(r), Some(w)) => Some(r | w),
        (r, w) => r.or(w),
    }
}

fn set_interest(registry: &Registry, stream: &mut TcpStream, token: Token, current: &mut Option<Interest>, wanted: Option<Interest>) -> Result<()> {
    if *current == wanted {
        return Ok(());
    }

    match (*current, wanted) {
        (Some(_), Some(interest)) => registry.reregister(stream, token, interest)?,
        (None, Some(interest)) => registry.register(stream, token, interest)?,
        (_, None) => registry.deregister(stream)?,
    }

    *current = wanted;
    Ok(())
}

/**
    Writes as much as the socket accepts. Returns [None] when it would block
*/
fn write_some(to: &mut TcpStream, data: &[u8]) -> Result<Option<usize>> {
    loop {
        match to.write(data) {
            Ok(0) => return Err(ErrorKind::WriteZero.into()),
            Ok(n) => return Ok(Some(n)),
            Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(None),
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
}
