use std::{
    ptr::{NonNull,null},
    cell::{RefCell,LazyCell},
    ops::{DerefMut},
    collections::VecDeque,
};
use mozjs::{rooted};
use mozjs::{
    realm::{AutoRealm},
    context::{JSContext},
    jsapi::{Heap,JSObject},
    jsval::{UndefinedValue,ObjectValue},
    gc::{Handle},
};
#[allow(unused_imports)] use tracing::{trace,debug,info,warn,error,instrument};

use super::incumbent_stack::{enter_incumbent_stack};

thread_local! {
    static QUEUE: LazyCell<RefCell<VecDeque<Task>>> = LazyCell::new(|| RefCell::new(VecDeque::new()));
}

/// Insert an item into the FIFO runtime queue
pub fn insert_into_filo(job: Box<Heap<*mut JSObject>>, global: Box<Heap<*mut JSObject>>) {
    QUEUE.with(|q| {
        q.borrow_mut().push_back(Task { job, obj: global });
    });
}

/// Remoe a item into the FIFO runtime queue
pub fn remove_from_filo() -> Option<Task> {
    QUEUE.with(|q| {
        q.borrow_mut().pop_front()
    })
}

pub fn filo_empty() -> bool {
    QUEUE.with(|q| {
        q.borrow().is_empty()
    })
}

/// Task contains everyting it needs to setup and run its job
pub struct Task {
    job: Box<Heap<*mut JSObject>>,
    obj: Box<Heap<*mut JSObject>>
}
impl Task {
    #[instrument(skip_all)]
    pub fn call(self, ctx: &mut JSContext) {
        rooted!(in(unsafe { ctx.raw_cx() }) let globals = self.obj.get());
        enter_incumbent_stack(ctx, globals.handle(), |realm: &mut AutoRealm, _ :Handle<'_,*mut JSObject>| -> () {
            //push_incumbent_stack(Heap::boxed(self.obj.get()));
            //let mut realm = AutoRealm::new(ctx, NonNull::new(self.obj.get()).unwrap());
            //let (_globals, realm) = realm.global_and_reborrow();
            rooted!(in(unsafe { realm.deref_mut().raw_cx() } ) let callback = ObjectValue(self.job.get()));
            rooted!(in(unsafe { realm.deref_mut().raw_cx() } ) let mut rval = UndefinedValue());
            let args = mozjs::jsapi::HandleValueArray {
                length_: 0,
                elements_: null(),
            };
            unsafe {
                let _ = mozjs::jsapi::JS::Call(
                    realm.deref_mut().raw_cx(),
                    mozjs::gc::HandleValue::undefined().into(),
                    callback.handle().into(),
                    &args,
                    rval.handle_mut().into(),
                );
            }
        });
        //pop_incumbent_stack();
    }
}

