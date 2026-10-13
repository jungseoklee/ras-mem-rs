use crate::core::instance::Instance;
use crate::core::{context::Context, handler::EventHandler};
use crate::event::mc::McEventHandler;

pub fn all_handlers<C: Context>() -> Vec<Box<dyn EventHandler<C>>> {
    let v: Vec<Box<dyn EventHandler<C>>> = vec![Box::new(McEventHandler)];
    v
}

pub fn handlers_for<C: Context>(instance: Instance) -> Vec<Box<dyn EventHandler<C>>> {
    all_handlers()
        .into_iter()
        .filter(|h| h.instance() == instance)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, core::instance::Instance, state::State};

    struct TestContext {
        config: Config,
        state: State,
    }
    impl Context for TestContext {
        fn config(&self) -> &Config {
            &self.config
        }
        fn state(&self) -> &State {
            &self.state
        }
    }

    #[test]
    fn test_all_handlers() {
        assert!(!all_handlers::<TestContext>().is_empty());
    }

    #[test]
    fn test_handlers_for() {
        assert_eq!(handlers_for::<TestContext>(Instance::Dram).len(), 1);
        assert!(handlers_for::<TestContext>(Instance::Hbm).is_empty());
    }

    #[test]
    fn test_coverage_workaround() {
        let ctx = TestContext {
            config: Config::default(),
            state: State::default(),
        };
        let _ = ctx.config();
        let _ = ctx.state();
    }
}
