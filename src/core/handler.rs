use crate::{
    core::{context::Context, instance::Instance},
    event::Event,
    traceevent::tep::{TepEvent, TepHandle, TepRecord},
};

pub trait EventHandler<C: Context>: Send + Sync {
    fn group(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn instance(&self) -> Instance;
    fn parse(
        &self,
        tep: &TepHandle,
        event: &TepEvent,
        record: &TepRecord<'_>,
        ctx: &C,
    ) -> Result<Event, String>;
    fn process(&self, event: &Event, ctx: &C);
}
