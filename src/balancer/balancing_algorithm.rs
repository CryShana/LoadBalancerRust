use std::net::SocketAddr;

pub trait BalancingAlgorithm: Sync + Send {
    fn get_next_host(&mut self) -> SocketAddr;
    fn report_error(&mut self, addr: SocketAddr);
}
