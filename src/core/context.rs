use crate::{config::Config, state::State};

pub trait Context: Send + Sync + 'static {
    fn config(&self) -> &Config;
    fn state(&self) -> &State;
}
