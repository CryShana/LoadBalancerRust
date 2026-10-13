use std::collections::HashMap;
use std::fmt::Arguments;
use std::io::{ErrorKind, Result};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use mio::net::TcpStream;
use mio::{Events, Poll, Token, Waker};

use super::{BalancingAlgorithm, TcpClient};

// timeout for a single connection attempt to a target host, after which the next host is tried
const CONNECTION_TIMEOUT: Duration = Duration::from_millis(400);

// total time a client may wait for any target host to accept it before it is disconnected
const TOTAL_CONNECTION_TIMEOUT: Duration = Duration::from_secs(4);

// connected clients with no socket activity for this long are disconnected
const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

// how often clients are checked for timeouts
const TIMEOUT_CHECK_INTERVAL: Duration = Duration::from_millis(50);

const WAKER_TOKEN: Token = Token(0);
const BUFFER_SIZE: usize = 16 * 1024;

type SharedAlgorithm = Arc<Mutex<dyn BalancingAlgorithm>>;

pub struct LoadBalancer {
    workers: Vec<WorkerHandle>,
}

struct WorkerHandle {
    sender: Sender<TcpStream>,
    waker: Waker,
    client_count: Arc<AtomicUsize>,
}

impl LoadBalancer {
    pub fn start(algorithm: impl BalancingAlgorithm + 'static, threads: usize, verbose: bool) -> Result<Self> {
        let algorithm: SharedAlgorithm = Arc::new(Mutex::new(algorithm));
        let mut workers = Vec::with_capacity(threads);

        // WORKERS
        for id in 0..threads {
            let poll = Poll::new()?;
            let waker = Waker::new(poll.registry(), WAKER_TOKEN)?;
            let (sender, receiver) = mpsc::channel();
            let client_count = Arc::new(AtomicUsize::new(0));

            let worker = Worker {
                ctx: Context {
                    id,
                    poll,
                    algorithm: Arc::clone(&algorithm),
                    verbose,
                },
                receiver,
                clients: HashMap::new(),
                next_token: WAKER_TOKEN.0 + 1,
                client_count: Arc::clone(&client_count),
            };
            thread::spawn(move || worker.run());

            workers.push(WorkerHandle { sender, waker, client_count });
        }

        Ok(LoadBalancer { workers })
    }

    /**
        Hands the client over to the worker thread with the fewest clients
    */
    pub fn add_client(&self, stream: std::net::TcpStream) {
        if let Err(e) = stream.set_nonblocking(true) {
            eprintln!("[Listener] Failed to set client socket to non-blocking: {}", e);
            return;
        }

        // find worker with least clients first
        let (id, worker) = self
            .workers
            .iter()
            .enumerate()
            .min_by_key(|(_, w)| w.client_count.load(Ordering::Relaxed))
            .expect("load balancer has no worker threads");

        // counted here instead of in the worker so a burst of new clients is spread across threads
        worker.client_count.fetch_add(1, Ordering::Relaxed);
        if worker.sender.send(TcpStream::from_std(stream)).is_err() || worker.waker.wake().is_err() {
            eprintln!("[Thread {}] Worker thread is not running, dropping client", id);
        }
    }
}

struct Worker {
    ctx: Context,
    receiver: Receiver<TcpStream>,
    clients: HashMap<Token, TcpClient>,
    next_token: usize,
    client_count: Arc<AtomicUsize>,
}

impl Worker {
    fn run(mut self) {
        let mut events = Events::with_capacity(1024);
        let mut buffer = vec![0; BUFFER_SIZE];
        let mut last_timeout_check = Instant::now();

        loop {
            // -------------------------------
            // EVENT POLLING
            // -------------------------------
            if let Err(e) = self.ctx.poll.poll(&mut events, Some(TIMEOUT_CHECK_INTERVAL)) {
                if e.kind() == ErrorKind::Interrupted {
                    continue;
                }

                eprintln!("[Thread {}] Failed to poll for events: {}", self.ctx.id, e);
                return;
            }

            // -------------------------------
            // EVENT LOOP (new clients and client/target readiness)
            // -------------------------------
            for event in events.iter() {
                match event.token() {
                    WAKER_TOKEN => self.accept_pending(),
                    token => self.handle_event(token, &mut buffer),
                }
            }

            // -------------------------------
            // CLIENT CHECKING (timeout handling)
            // -------------------------------
            if last_timeout_check.elapsed() >= TIMEOUT_CHECK_INTERVAL {
                self.check_timeouts();
                last_timeout_check = Instant::now();
            }
        }
    }

    fn accept_pending(&mut self) {
        while let Ok(stream) = self.receiver.try_recv() {
            let token = Token(self.next_token);
            self.next_token += 1;

            match TcpClient::new(stream, token, self.ctx.poll.registry()) {
                Ok(mut client) => {
                    self.ctx.log(format_args!("Connected from {}", client.address));
                    self.ctx.connect(&mut client);
                    self.clients.insert(token, client);
                }
                Err(e) => {
                    self.ctx.log(format_args!("Failed to register client: {}", e));
                    self.client_count.fetch_sub(1, Ordering::Relaxed);
                }
            }
        }
    }

    fn handle_event(&mut self, token: Token, buffer: &mut [u8]) {
        let Some(client) = self.clients.get_mut(&token) else {
            return;
        };

        if !self.ctx.process(client, buffer) {
            self.remove(token);
        }
    }

    fn check_timeouts(&mut self) {
        let expired: Vec<Token> = self
            .clients
            .iter_mut()
            .filter_map(|(&token, client)| (!self.ctx.check_timeout(client)).then_some(token))
            .collect();

        for token in expired {
            self.remove(token);
        }
    }

    fn remove(&mut self, token: Token) {
        if let Some(mut client) = self.clients.remove(&token) {
            client.close(self.ctx.poll.registry());
            self.client_count.fetch_sub(1, Ordering::Relaxed);
            self.ctx.log(format_args!(
                "Connection ended ({}) [Remaining clients: {}]",
                client.address,
                self.clients.len()
            ));
        }
    }
}

/**
    Worker state that is used while a client is borrowed from the worker's client map
*/
struct Context {
    id: usize,
    poll: Poll,
    algorithm: SharedAlgorithm,
    verbose: bool,
}

impl Context {
    /**
        Handles a readiness event for the client. Returns false when the client should be removed
    */
    fn process(&self, client: &mut TcpClient, buffer: &mut [u8]) -> bool {
        if !client.is_connected() {
            match client.check_connected() {
                Ok(true) => {
                    let target = client.target_addr().unwrap();
                    self.log(format_args!("Client connected to target ({} -> {})", client.address, target));
                    self.algorithm.lock().unwrap().report_success(target);
                }
                Ok(false) => return true,
                Err(e) => {
                    self.connect_failed(client, format_args!("Failed to connect to target: {}", e));
                    return true;
                }
            }
        }

        match client.forward(buffer, self.poll.registry()) {
            Ok(open) => open,
            Err(e) => {
                self.log(format_args!("Connection error ({}): {}", client.address, e));
                false
            }
        }
    }

    /**
        Retries or drops clients that are taking too long to connect, and drops idle ones. Returns false when the client should be removed
    */
    fn check_timeout(&self, client: &mut TcpClient) -> bool {
        if client.is_connected() {
            if client.last_activity.elapsed() > IDLE_TIMEOUT {
                self.log(format_args!("Idle timeout ({})", client.address));
                return false;
            }

            return true;
        }

        if client.accepted_at.elapsed() > TOTAL_CONNECTION_TIMEOUT {
            self.log(format_args!("Timed out, no target host available ({})", client.address));
            return false;
        }

        match client.target_started_at() {
            Some(started_at) if started_at.elapsed() > CONNECTION_TIMEOUT => self.connect_failed(client, format_args!("Connection to target timed out")),
            Some(_) => {}
            None => self.connect(client),
        }

        true
    }

    fn connect(&self, client: &mut TcpClient) {
        let target = self.algorithm.lock().unwrap().get_next_host();
        self.log(format_args!("Connecting client ({} -> {})", client.address, target));

        // on failure the target is left unset and the next timeout check retries with another host
        if let Err(e) = client.connect(target, self.poll.registry()) {
            self.log(format_args!("Failed to connect to target {}: {}", target, e));
            self.algorithm.lock().unwrap().report_error(target);
        }
    }

    fn connect_failed(&self, client: &mut TcpClient, reason: Arguments) {
        let target = client.target_addr().unwrap();
        self.log(format_args!("{} ({} -> {})", reason, client.address, target));
        self.algorithm.lock().unwrap().report_error(target);
        self.connect(client);
    }

    fn log(&self, args: Arguments) {
        if self.verbose {
            println!("[Thread {}] {}", self.id, args);
        }
    }
}
