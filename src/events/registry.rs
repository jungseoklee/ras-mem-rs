use crate::core::{context::Context, handler::EventHandler};
use crate::events::mc::McEventHandler;

pub fn all_handlers<C: Context>() -> Vec<Box<dyn EventHandler<C>>> {
    let v: Vec<Box<dyn EventHandler<C>>> = vec![Box::new(McEventHandler)];
    v
}
