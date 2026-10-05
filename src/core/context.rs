use crate::{config::Config, states::State};

pub trait Context: Send + Sync + 'static {
    fn config(&self) -> &Config;
    fn state(&self) -> &State;
}
