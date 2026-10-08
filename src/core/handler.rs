use crate::{
    core::{context::Context, instance::Instance},
    event::Event,
    traceevent::{
        tep::{TepEvent, TepHandle, TepRecord},
        trace_seq::TraceSeq,
    },
};

pub trait EventHandler<C: Context>: Send + Sync {
    fn group(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn instance(&self) -> Instance;
    fn parse(
        &self,
        tep: &TepHandle<C>,
        seq: &mut TraceSeq,
        event: &TepEvent,
        record: &TepRecord,
        ctx: &C,
    ) -> Result<Event, String>;
    fn update_state(&self, event: Event, ctx: &C);
}
