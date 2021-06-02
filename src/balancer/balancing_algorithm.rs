pub trait BalancingAlgorithm {
    fn get_next_host(&mut self) -> String;
}