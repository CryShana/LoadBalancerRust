use std::net::SocketAddr;

pub trait BalancingAlgorithm {
    fn get_next_host(&mut self) -> SocketAddr;
}