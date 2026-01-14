use std::{
    cell::{Cell},
};
use mozjs::{
    context::{JSContext},
};
use super::{
    queue::{remove_from_filo,filo_empty},
    resolvable_promise::{futures_empty, setup_to_resolve, poll_futures},
};

thread_local! {
    static CHECKPOINT: Cell<bool> = Cell::new(false);
}

pub fn set_checkpoint(b: bool) {
    CHECKPOINT.with(|c| c.set(b));
}

pub fn get_checkpoint() -> bool {
    CHECKPOINT.with(|c| c.get())
}


pub fn runtime_checkpoint(ctx: &mut JSContext) {
    if get_checkpoint() {
        // function must be rrentrant
        return;
    }
    set_checkpoint(true);

    loop {
        while let Some(task) = remove_from_filo() {
            task.call(ctx);
        }

        for (key,lambda)in poll_futures() {
            setup_to_resolve(ctx, key, lambda);
        }

        if futures_empty() && filo_empty() {
            break;
        }
    }

    set_checkpoint(false);
}

pub fn is_empty() -> bool {
    futures_empty() && filo_empty()
}
