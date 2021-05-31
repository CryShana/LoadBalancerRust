use std::io::prelude::*;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use std::thread::Thread;
use std::{
    net::{IpAddr, SocketAddr, TcpStream},
    thread,
    time::Duration,
    u16,
};

use super::TcpClient;
const CONNECTION_TIMEOUT: Duration = Duration::from_millis(1500);
const SLEEP_TIME: Duration = Duration::from_millis(5);

pub struct LoadBalancer {
    clients: Arc<RwLock<Vec<TcpClient>>>,
    stopped: Arc<RwLock<bool>>,
    threads: u16,
    workers: Vec<Thread>,
}

impl LoadBalancer {
    pub fn new(threads: u16) -> Self {
        let mut b = LoadBalancer {
            clients: Arc::new(RwLock::new(vec![])),
            stopped: Arc::new(RwLock::new(false)),
            threads: threads,
            workers: vec![],
        };

        b.spawn_workers();

        b
    }

    pub fn add_client(&mut self, stream: TcpStream) {
        let client = TcpClient::new(stream);
        self.clients.write().unwrap().push(client);
    }

    pub fn stop(&mut self) {
        *self.stopped.write().unwrap() = true;
    }

    fn handle_client(mut client: TcpClient) {
        let target_port: u16 = 5000;
        let target_addr: IpAddr = IpAddr::from_str("127.0.0.1").unwrap();
        let target_socket = SocketAddr::new(target_addr, target_port);

        client.connect_to_target(target_socket, CONNECTION_TIMEOUT);

        loop {
            client.process();
            thread::sleep(SLEEP_TIME);

            break;
        }
    }

    fn spawn_workers(&mut self) {
        for id in 0..self.threads {
            let c = Arc::clone(&self.clients);
            let s = Arc::clone(&self.stopped);
            let threads = self.threads;

            thread::spawn(move || {
                {
                    let clients = &*c.read().unwrap();
                }

                println!("Thread ID -> {}", id);

                {
                    let stopped = *s.read().unwrap();
                    if stopped == true {
                        return;
                    }
                }
            });
        }
    }
}
