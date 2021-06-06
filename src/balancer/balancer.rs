use std::collections::HashMap;
use std::io::ErrorKind;
use std::sync::Arc;
use std::sync::RwLock;
use std::usize;
use std::vec;
use std::{thread, time::Duration, u16};

use super::BalancingAlgorithm;
use super::RoundRobin;
use super::TcpClient;
use mio::net::TcpStream;
use mio::Events;
use mio::Interest;
use mio::Poll;
use mio::Token;

// this is used as the total timeout allowed to connect before client is disconnected
const TOTAL_CONNECTION_TIMEOUT: Duration = Duration::from_millis(4000);

// this is used as the timeout to connect to a target host
const CONNECTION_TIMEOUT: Duration = Duration::from_millis(400);

pub struct LoadBalancer {
    /**
        Holds client counts for all threads
    */
    client_counts: Arc<RwLock<Vec<Arc<RwLock<usize>>>>>,
    /**
        Newly added clients are added here, threads will add them to polling when they can
    */
    client_lists_pending: Arc<RwLock<Vec<Arc<RwLock<Vec<TcpClient>>>>>>,
    threads: u16,
    stopped: Arc<RwLock<bool>>,
    debug: Arc<RwLock<bool>>,
    balancing_algorithm: Arc<RwLock<RoundRobin>>,
}

impl LoadBalancer {
    pub fn new(balancing_algorithm: RoundRobin, threads: u16, debug: bool) -> Self {
        // prepare client lists for every thread
        let mut client_counts: Vec<Arc<RwLock<usize>>> = vec![];
        for _ in 0..threads {
            client_counts.push(Arc::new(RwLock::new(0)));
        }
        let client_counts = Arc::new(RwLock::new(client_counts));

        // prepare pending client lists for every thread
        let mut client_lists_pending: Vec<Arc<RwLock<Vec<TcpClient>>>> = vec![];
        for _ in 0..threads {
            let lists: Vec<TcpClient> = vec![];
            client_lists_pending.push(Arc::new(RwLock::new(lists)));
        }
        let client_lists_pending = Arc::new(RwLock::new(client_lists_pending));

        let b = LoadBalancer {
            client_counts,
            client_lists_pending,
            threads,
            stopped: Arc::new(RwLock::new(false)),
            debug: Arc::new(RwLock::new(debug)),
            balancing_algorithm: Arc::new(RwLock::new(balancing_algorithm)),
        };

        b
    }

    pub fn start(&mut self) {
        self.spawn_threads();
    }

    pub fn add_client(&mut self, stream: TcpStream) {
        let client = TcpClient::new(stream);

        // pick client list with least clients and add it to pending list
        let client_counts = self.client_counts.read().unwrap();
        let client_lists_pending = self.client_lists_pending.read().unwrap();

        // find client list with least clients first
        let mut min_index = 0;
        let mut min_length = *client_counts[0].read().unwrap();
        for i in 1..client_counts.len() {
            let len = *client_counts[i].read().unwrap();
            if len < min_length {
                min_length = len;
                min_index = i;
            }
        }

        if *self.debug.read().unwrap() {
            println!("[Thread {}] Connected from {}", min_index, client.address);
        }

        // add client to pending list
        client_lists_pending[min_index].write().unwrap().push(client);
    }

    pub fn stop(&mut self) {
        *self.stopped.write().unwrap() = true;
    }

    fn spawn_threads(&mut self) {
        let th = self.threads as u32;

        // WORKERS
        for id in 0..th {
            let stopped = Arc::clone(&self.stopped);
            let d = Arc::clone(&self.debug);
            let b = Arc::clone(&self.balancing_algorithm);
            let client_counts = Arc::clone(&self.client_counts);
            let client_list_pending = Arc::clone(&self.client_lists_pending);

            thread::spawn(move || {
                let mut connected_sockets: HashMap<Token, TcpClient> = HashMap::new();
                let mut next_token_id: usize = 0;

                let mut get_next_token = || {
                    let token = Token(next_token_id);
                    next_token_id += 1;
                    if next_token_id >= usize::MAX {
                        next_token_id = 1;
                    }
                    token
                };

                let client_list_index = id as usize;

                let mut poll = Poll::new().unwrap();
                let mut events = Events::with_capacity(1024);

                loop {
                    // keep checking if balancer has been stopped
                    if *stopped.read().unwrap() {
                        break;
                    }

                    // -------------------------------
                    // EVENT POLLING
                    // -------------------------------
                    match poll.poll(&mut events, Some(Duration::from_millis(10))) {
                        Ok(_) => {}
                        Err(ref e) if e.kind() == ErrorKind::Interrupted => {
                            // this handler does not get called on Windows, so we use timeout and check it outside
                            *stopped.write().unwrap() = true;
                        }
                        Err(e) => {
                            println!("[Thread {}] Failed to poll for events! {}", id, e.to_string());
                            break;
                        }
                    };

                    // -------------------------------
                    // PROCESS PENDING CLIENTS
                    // -------------------------------
                    {
                        // check if any pending clients (try to read to avoid blocking)
                        let r: i32 = match client_list_pending.read().unwrap()[client_list_index].try_read() {
                            Ok(r) => r.len() as i32,
                            Err(_) => -1,
                        };
                        if r > 0 {
                            let p_list = &*client_list_pending.read().unwrap()[client_list_index];

                            let pending = &mut *match p_list.try_write() {
                                Ok(w) => w,
                                Err(_) => continue,
                            };

                            // move all pending clients over to our client_list and register them with poll
                            let plen = pending.len();
                            for i in 0..plen {
                                let index = (plen - 1) - i;
                                let mut client = pending.remove(index);

                                let token = get_next_token();

                                poll.registry().register(&mut client.stream, token, Interest::READABLE).unwrap();

                                // insert into hashmap for quick lookup
                                connected_sockets.insert(token, client);
                            }

                            // update count
                            *client_counts.read().unwrap()[client_list_index].write().unwrap() = connected_sockets.len();
                        }
                    }

                    // -------------------------------
                    // CLIENT CHECKING (timeout handling)
                    // -------------------------------
                    {
                        // check for connecting clients for time outs and their current state
                        let mut tokens_to_remove: Vec<Box<Token>> = vec![];
                        for (token, client) in &mut connected_sockets {
                            // if client not connected, schedule for removal
                            if !client.is_client_connected() {
                                let t = Box::new(token.clone());
                                tokens_to_remove.push(t);
                                continue;
                            }

                            // if client not in IN_CONNECTING state, we can't check for time outs
                            if !client.is_connecting() {
                                continue;
                            }

                            // HANDLE TIMEOUT TO SINGLE TARGET
                            if client.started_connecting.elapsed() > CONNECTION_TIMEOUT {
                                if *d.read().unwrap() {
                                    println!(
                                        "[Thread {}] Connection to target timed out ({} <-> {})",
                                        id,
                                        client.address,
                                        client.get_target_addr().unwrap()
                                    );
                                }

                                // we timed out! Let's try another host
                                client.close_connection_to_target(true);
                                LoadBalancer::report_target_error(client, Arc::clone(&b));
                                LoadBalancer::start_connection(id, token.clone(), client, &poll, Arc::clone(&d), Arc::clone(&b));
                            }

                            // HANDLE TOTAL TIMEOUT
                            if client.last_connection_loss.elapsed() > TOTAL_CONNECTION_TIMEOUT {
                                if *d.read().unwrap() {
                                    println!("[Thread {}] Timed out ({})", id, client.address);
                                }

                                // we timed out completely!
                                client.close_connection();
                            }
                        }

                        // now remove the marked clients
                        if tokens_to_remove.len() > 0 {
                            for token in tokens_to_remove {
                                let mut client = connected_sockets.remove(&token).unwrap();
                                poll.registry().deregister(&mut client.stream).unwrap();

                                if *d.read().unwrap() {
                                    println!(
                                        "[Thread {}] Connection ended ({}) [Remaining clients: {}]",
                                        id,
                                        client.address,
                                        connected_sockets.len()
                                    );
                                }
                            }

                            // update count
                            *client_counts.read().unwrap()[client_list_index].write().unwrap() = connected_sockets.len();
                        }
                    }

                    // ------------------------------
                    // EVENT LOOP
                    // ------------------------------
                    if events.is_empty() || *stopped.read().unwrap() {
                        continue;
                    }
                    for event in events.iter() {
                        match event.token() {
                            token => {
                                let client = match connected_sockets.get_mut(&token) {
                                    Some(c) => c,
                                    None => {
                                        println!("ERROR - Tried getting client that was not present in hash map! -> token: {:?}", token);
                                        // TODO: maybe deregister from poll if this is ever even called
                                        continue;
                                    }
                                };

                                if !client.is_client_connected() {
                                    // ignore, will be handled in later loop and cleaned
                                    continue;
                                }

                                // if client is in process of connecting, check if connection has been established
                                if client.is_connecting() {
                                    LoadBalancer::try_confirm_connection(id, client, Arc::clone(&d), Arc::clone(&b));
                                }

                                // if connected, process it normally, otherwise start a new connection to next host
                                if client.is_connected() {
                                    LoadBalancer::process_client(client, Arc::clone(&b));
                                } else if !client.is_connecting() {
                                    LoadBalancer::start_connection(id, token, client, &poll, Arc::clone(&d), Arc::clone(&b));
                                }
                            }
                        }
                    }
                }
            });
        }
    }

    fn try_confirm_connection(id: u32, client: &mut TcpClient, d: Arc<RwLock<bool>>, b: Arc<RwLock<RoundRobin>>) {
        let server_connected = client.check_target_connected().unwrap_or_else(|e| {
            println!("Not connected unknown error -> {}", e.to_string());
            // TODO: should probably disconnect - there was an error while connecting other than NotConnected
            false
        });

        if server_connected {
            let addr = client.get_target_addr().unwrap();

            if *d.read().unwrap() && !client.is_connecting() {
                println!("[Thread {}] Client connected ({} -> {})", id, client.address, addr);
            }

            // report success if connection succeeded
            if b.read().unwrap().is_on_cooldown(addr) {
                b.write().unwrap().report_success(addr);
            }
        }
    }

    fn process_client(client: &mut TcpClient, b: Arc<RwLock<RoundRobin>>) {
        let success = client.process();

        if success == false {
            // connection to either server or client has failed

            // removal from list is handled later

            LoadBalancer::report_target_error(client, Arc::clone(&b));
        }
    }

    fn start_connection(id: u32, token: Token, client: &mut TcpClient, poll: &Poll, d: Arc<RwLock<bool>>, b: Arc<RwLock<RoundRobin>>) {
        // determine target host to connect to, using the balancing algorithm!
        let target_socket = match client.get_target_addr() {
            Some(s) => s,
            None => b.write().unwrap().get_next_host(),
        };

        if *d.read().unwrap() && !client.is_connecting() {
            println!("[Thread {}] Connecting client ({} -> {})", id, client.address, target_socket);
        }

        // connect to target
        let success = match client.connect_to_target(target_socket) {
            Ok(s) => s,
            Err(e) => {
                println!(
                    "[Thread {}] Unexpected error while trying to start a connection! {} ({} -> {})",
                    id,
                    e.to_string(),
                    client.address,
                    target_socket
                );
                false
            }
        };

        if success {
            // connection to target host started
            // add server to poll (with same token as client)
            client.register_target_with_poll(&poll, token);
        } else {
            // report host error to host manager
            LoadBalancer::report_target_error(client, Arc::clone(&b));
        }
    }

    fn report_target_error(client: &mut TcpClient, b: Arc<RwLock<RoundRobin>>) {
        // report host error to host manager
        let last_t = client.get_last_target_addr();
        if client.last_target_errored() && last_t.is_some() {
            b.write().unwrap().report_error(last_t.unwrap());
        }
    }
}
