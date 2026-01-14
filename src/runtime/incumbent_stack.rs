use std::{
    ptr::{NonNull},
    cell::{LazyCell,RefCell},
};
use mozjs::{
    context::{JSContext},
    panic::{wrap_panic},
    gc::{MutableHandle,Handle},
    jsapi::{Heap,JSObject},
    realm::AutoRealm,
};
#[allow(unused_imports)] use tracing::{trace,debug,info,warn,error,instrument};

thread_local! {
    static INCUMBENT_STACK: LazyCell<RefCell<Vec<Box<Heap<*mut JSObject>>>>> = LazyCell::new(|| RefCell::new(Vec::new()));
}

/// Pushes an item into the incumbent stack
pub(crate) fn push_incumbent_stack(item: Box<Heap<*mut JSObject>>) {
    INCUMBENT_STACK.with(|inner| {
        inner.borrow_mut().push(item);
    })
}
/// Pops an item from the incumbent stack
#[instrument(skip_all)]
fn pop_incumbent_stack() -> Option<Box<Heap<*mut JSObject>>> {
    INCUMBENT_STACK.with(|inner| {
        let out = inner.borrow_mut().pop();
        if out.is_none() {
            warn!("incumbent stack is empty");
        }
        out
    })
}

/// Peak at what is on the top of our stack
#[instrument(skip_all)]
pub(crate) fn peek_incumbent_stack(target: &mut MutableHandle<'_,*mut JSObject>) {
    INCUMBENT_STACK.with(|inner| {
        match inner.borrow_mut().last() {
            None => {
                error!("no incumbent stack is present");
            }
            Some(inner) => {
                target.set(inner.get());
            }
        };
    });
}

#[instrument(skip_all, name = "incubment stack frame")]
pub fn enter_incumbent_stack<F,R>(ctx: &mut JSContext, globals: Handle<'_, *mut JSObject>, lambda: F) -> R
where
    F: FnOnce(&mut AutoRealm, Handle<'_,*mut JSObject>) -> R,
{
    push_incumbent_stack(Heap::boxed(globals.get()));
    let mut realm = AutoRealm::new(ctx, NonNull::new(globals.get()).unwrap());
    let mut out: Option<R> = None;
    out = Some((lambda)(&mut realm, globals));
    pop_incumbent_stack();
    out.unwrap()
}

