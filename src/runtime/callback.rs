use std::{
    ptr::{null,NonNull,null_mut},
    ffi::{c_void},
};
use mozjs::{rooted};
use mozjs::{
    JSCLASS_RESERVED_SLOTS_MASK,
    context::{JSContext},
    jsapi::{
        HandleObject,
        JSObject,Heap,Value,
        JSCLASS_RESERVED_SLOTS_SHIFT, JSClassOps, JSClass,
    },
    panic::{wrap_panic},
    jsval::{UndefinedValue,ObjectValue},
    glue::JobQueueTraps,
};

use super::{
    incumbent_stack::{peek_incumbent_stack},
    queue::{insert_into_filo},
    checkpoint::{runtime_checkpoint,is_empty},
};
#[allow(unused_imports)] use tracing::{trace,debug,info,warn,error,instrument};

/*
 * Define Callbacks & Global State
 *
 */
unsafe extern "C" fn get_host_defined_data(
    _extra: *const c_void,
    ctx: *mut mozjs::context::RawJSContext,
    data: mozjs::jsapi::MutableHandleObject,
)-> bool {

    /*
     * Define a custom object to
     * hold our global references
     *
     * This is what servo does, so I assume(?)
     * it is correct(?)
     *
     * TODO: try just manually stuffing the global
     *       within `data`
     */

    static HOST_DEFINED_DATA_OPS: JSClassOps = JSClassOps {
        addProperty: None,
        delProperty: None,
        enumerate: None,
        newEnumerate: None,
        resolve: None,
        mayResolve: None,
        finalize: None,
        call: None,
        construct: None,
        trace: None,
    };
    static HOST_DEFINED_DATA_CLASS: JSClass = JSClass {
        name: c"HostDefinedData".as_ptr(),
        flags: (1 & JSCLASS_RESERVED_SLOTS_MASK) << JSCLASS_RESERVED_SLOTS_SHIFT,
        cOps: &HOST_DEFINED_DATA_OPS,
        spec: null(),
        ext: null(),
        oOps: null(),
    };

    let mut is_okay = false;
    wrap_panic(&mut || {
        rooted!(in(ctx) let mut incumbent_stack = null_mut::<JSObject>());
        peek_incumbent_stack(&mut incumbent_stack.handle_mut());
        let guard = if incumbent_stack.get().is_null() {
            warn!("incumbent stack has no items to peek");
            rooted!(in(ctx) let current = unsafe { mozjs::jsapi::CurrentGlobalOrNull(ctx) });
            if current.get().is_null() {
                error!("incumbent stack has nothing and there is no current global");
                is_okay = false;
                return;
            }
            Heap::boxed(current.get())
        } else {
            Heap::boxed(incumbent_stack.get())
        };
        rooted!(in(ctx) let result = unsafe { mozjs::jsapi::JS_NewObject(ctx, &HOST_DEFINED_DATA_CLASS) });
        if result.get().is_null() {
            is_okay = false;
            error!("could not setup globals");
            return;
        }
        rooted!(in(ctx) let out = ObjectValue(guard.get()));
        unsafe { mozjs::jsapi::JS_SetReservedSlot(result.get(),0,out.as_ptr() as *const Value) };
        data.set(result.get());
        is_okay = true;
    });
    is_okay
}



/// Called once a promise has resolved/rejected
///
/// As I understand it we're being passed a callback (job)
/// to be scheduled at a future data.
unsafe extern "C" fn enqueue_promise_job(
    _extra: *const c_void,
    cx: *mut mozjs::context::RawJSContext,
    _promise: HandleObject,
    job: HandleObject,
    _allocation_site: HandleObject,
    host_defined_data: HandleObject,
) -> bool {
    wrap_panic(&mut || {
        rooted!(in(cx) let host_data = UndefinedValue());
        if !host_defined_data.get().is_null() {
            unsafe { mozjs::glue::JS_GetReservedSlot(host_defined_data.get(), 0, host_data.as_ptr()) }
        }
        if !host_data.is_undefined() && !host_data.is_null() && host_data.is_object() {
            rooted!(in(cx) let incumbent_obj = host_data.to_object());
            if incumbent_obj.get().is_null() {
                warn!("incumbent stack item is null pointer");
            }
            insert_into_filo(Heap::boxed(job.get()), Heap::boxed(incumbent_obj.get()));
        }
    });
    true
}

unsafe extern "C" fn push_new_interrupt_queue(_: *mut c_void) -> *const c_void {
    null()
}

unsafe extern "C" fn pop_interrupt_queue(_: *mut c_void) -> *const c_void {
    null()
}

unsafe extern "C" fn drop_interrupt_queues(_: *mut c_void) {}

unsafe extern "C" fn run_jobs(_extra: *const c_void, cx: *mut mozjs::context::RawJSContext) {
    wrap_panic(&mut || {
        let mut ctx = unsafe { JSContext::from_ptr(NonNull::new(cx).unwrap()) };
        runtime_checkpoint(&mut ctx);
    });
}

unsafe extern "C" fn empty(_: *const c_void) -> bool {
    let mut runtime_empty = false;
    wrap_panic(&mut || {
        runtime_empty = is_empty()
    });
    runtime_empty
}

pub const JOB_QUEUE_TRAPS: JobQueueTraps = JobQueueTraps {
    getHostDefinedData: Some(get_host_defined_data),
    enqueuePromiseJob: Some(enqueue_promise_job),
    runJobs: Some(run_jobs),
    empty: Some(empty),
    pushNewInterruptQueue: Some(push_new_interrupt_queue),
    popInterruptQueue: Some(pop_interrupt_queue),
    dropInterruptQueues: Some(drop_interrupt_queues),
};


