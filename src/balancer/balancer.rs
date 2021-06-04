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
        Holds all clients for all threads
    */
    client_lists: Arc<RwLock<Vec<Arc<RwLock<Vec<TcpClient>>>>>>,
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
        let mut client_lists: Vec<Arc<RwLock<Vec<TcpClient>>>> = vec![];
        for _ in 0..threads {
            let lists: Vec<TcpClient> = vec![];
            client_lists.push(Arc::new(RwLock::new(lists)));
        }
        let client_lists = Arc::new(RwLock::new(client_lists));

        // prepare pending client lists for every thread
        let mut client_lists_pending: Vec<Arc<RwLock<Vec<TcpClient>>>> = vec![];
        for _ in 0..threads {
            let lists: Vec<TcpClient> = vec![];
            client_lists_pending.push(Arc::new(RwLock::new(lists)));
        }
        let client_lists_pending = Arc::new(RwLock::new(client_lists_pending));

        let b = LoadBalancer {
            client_lists,
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
        let client_lists = self.client_lists.read().unwrap();
        let client_lists_pending = self.client_lists_pending.read().unwrap();

        // find client list with least clients first
        let mut min_index = 0;
        let mut min_length = client_lists[0].read().unwrap().len();
        for i in 1..client_lists.len() {
            let list = client_lists[i].read().unwrap();
            let len = list.len();
            if len < min_length {
                min_length = len;
                min_index = i;
            }
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
            let client_list = Arc::clone(&self.client_lists);
            let client_list_pending = Arc::clone(&self.client_lists_pending);

            thread::spawn(move || {
                let client_list_index = id as usize;

                let mut poll = Poll::new().unwrap();
                let mut events = Events::with_capacity(1024);

                loop {
                    // keep checking if balancer has been stopped
                    if *stopped.read().unwrap() {
                        break;
                    }

                    // poll for client events
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

                    // check if any pending clients (try to read to avoid blocking)
                    let r: i32 = match client_list_pending.read().unwrap()[client_list_index].try_read() {
                        Ok(r) => r.len() as i32,
                        Err(_) => -1,
                    };
                    if r > 0 {
                        let p_list = &*client_list_pending.read().unwrap()[client_list_index];
                        let c_list = &*client_list.read().unwrap()[client_list_index];

                        let pending = &mut *match p_list.try_write() {
                            Ok(w) => w,
                            Err(_) => continue,
                        };

                        let target_list = &mut *c_list.write().unwrap();

                        // move all pending clients over to our client_list and register them with poll
                        let plen = pending.len();
                        for i in 0..plen {
                            let index = (plen - 1) - i;
                            let mut client = pending.remove(index);

                            poll.registry()
                                .register(&mut client.stream, Token(0), Interest::READABLE | Interest::WRITABLE)
                                .unwrap();

                            target_list.push(client);
                        }
                    }

                    if events.is_empty() || *stopped.read().unwrap() {
                        continue;
                    }

                    // TODO: optimize in the future so that only clients that are associated with an event, are processed!
                    let mut should_process = false;

                    // handle events
                    for event in events.iter() {
                        match event.token() {
                            _ => {
                                if event.is_readable() {}

                                if event.is_writable() {}

                                should_process = true;
                                break;
                            }
                        }
                    }

                    if !should_process {
                        continue;
                    }

                    // HANDLE CLIENTS
                    {
                        let c_list = &*client_list.read().unwrap()[client_list_index];
                        let mut list = c_list.write().unwrap();
                        let mut list_length: i32 = list.len() as i32;
                        let mut i: i32 = -1;
                        loop {
                            i = i + 1;
                            if i >= list_length {
                                break;
                            }

                            let client = &mut list[i as usize];

                            // remove clients that are no longer connected
                            if client.is_client_connected() == false {
                                list.remove(i as usize);
                                list_length = list_length - 1;
                                i = i - 1;
                                continue;
                            }

                            // handle client
                            if client.is_connected() {
                                let success = client.process();

                                if success == false {
                                    // connection to either server or client has failed

                                    // removal from list is handled later
                                    if *d.read().unwrap() {
                                        println!("[Thread {}] Connection ended ({})", id, client.address);
                                    }

                                    // report host error to host manager
                                    let last_t = client.get_last_target_addr();
                                    if client.last_target_errored() && last_t.is_some() {
                                        b.write().unwrap().report_error(last_t.unwrap());
                                    }
                                }
                            } else {
                                // determine target host to connect to, using the balancing algorithm!
                                let target_socket = match client.get_target_addr() {
                                    Some(s) => s,
                                    None => b.write().unwrap().get_next_host(),
                                };

                                if *d.read().unwrap() && !client.is_connecting() {
                                    println!("[Thread {}] Connecting client ({} -> {})", id, client.address, target_socket);
                                }

                                // connect to target
                                let success = match client.connect_to_target(target_socket, CONNECTION_TIMEOUT, TOTAL_CONNECTION_TIMEOUT) {
                                    Ok(s) => s,
                                    Err(e) => {
                                        println!(
                                            "[Thread {}] Unexpected error while trying to connect! {} ({} -> {})",
                                            id,
                                            e.to_string(),
                                            client.address,
                                            target_socket
                                        );
                                        false
                                    }
                                };

                                if success {
                                    // connection to target host succeeded!
                                    if *d.read().unwrap() {
                                        println!("[Thread {}] Client connected ({} -> {})", id, client.address, target_socket);
                                    }

                                    // TODO: add server to poll

                                    // report success if connection succeeded - we first check if it's even necessary before taking WRITE access for the balancer
                                    if b.read().unwrap().is_on_cooldown(target_socket) {
                                        b.write().unwrap().report_success(target_socket);
                                    }
                                } else {
                                    // report host error to host manager
                                    let last_t = client.get_last_target_addr();
                                    if client.last_target_errored() && last_t.is_some() {
                                        b.write().unwrap().report_error(last_t.unwrap());
                                    }
                                }
                            }
                        }
                    }
                }
            });
        }
    }
}
