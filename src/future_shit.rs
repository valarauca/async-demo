use std::{
    ops::DerefMut,
    ptr::{NonNull,null_mut},
    rc::{Rc},
};

use mozjs::{rooted};
use mozjs::{
    realm::{AutoRealm},
    gc::{Handle,MutableHandle,},
    jsapi::{JSObject,Value,CallArgs,Heap},
    jsval::{ObjectValue},
    context::{JSContext},
    panic::wrap_panic,
};
#[allow(unused_imports)]
use tracing::{debug,trace,instrument,warn,error,info};

use crate::runtime::resolvable_promise::{ResolutionMarshalling,Bridge,push_internal_promise,why_is_promise_id_segfaulting};

#[instrument(skip_all,name="tokio_sleep_entry_point")]
pub unsafe extern "C" fn tokio_sleep_ms(
    ctx: *mut mozjs::context::RawJSContext,
    argc: u32,
    vp: *mut Value,
) -> bool {
    let mut is_okay = true;
    wrap_panic(&mut || {
        let args = unsafe { CallArgs::from_vp(vp, argc) };
        if args.argc_ < 1 {
            error!("incorrect number of args");
            is_okay = false;
            return;
        }
        let ms = args.get(0).to_number();
        let duration_ms = ms as u64;
        let mut safe_ctx = unsafe {JSContext::from_ptr(NonNull::new(ctx).unwrap())};
        rooted!(in(unsafe { safe_ctx.raw_cx() }) let promise = unsafe { mozjs::rust::wrappers2::NewPromiseObject(&mut safe_ctx, Handle::<'_,*mut JSObject>::null()) });
        rooted!(in(unsafe { safe_ctx.raw_cx() }) let current_global = unsafe { mozjs::rust::wrappers2::CurrentGlobalOrNull(&safe_ctx)});
        if current_global.get().is_null() {
            is_okay = false;
            error!("global is null");
            return;
        }
        let promise_id = unsafe {
            if promise.get().is_null() {
				is_okay = false;
                error!("promise is null");
                return;
            }
            if !unsafe { mozjs::rust::wrappers2::IsPromiseObject(promise.handle()) } {
				is_okay = false;
                error!("promise is not a promise");
                return;
            }
            let promise_id = mozjs::rust::wrappers2::GetPromiseID(promise.handle());
            args.rval().set(ObjectValue(promise.get()));
            promise_id
		};

	    let task = tokio::task::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(duration_ms)).await;
            duration_ms
        });

	    push_internal_promise(promise_id, Heap::boxed(promise.get()), Rc::new(Heap::boxed(current_global.get())));
	    Bridge::new(async move { (promise_id,task.await.unwrap()) }, bridge_delay);
        info!("tokio sleep returning, state: '{}'", &is_okay);
    });
    is_okay
}

fn bridge_delay(x: u64) -> ResolutionMarshalling {
    use mozjs::conversions::ToJSValConvertible;
    Box::new(move |realm: &mut AutoRealm, _promise: Handle<'_, *mut JSObject>, _global: Handle<'_, *mut JSObject>, ok: MutableHandle<'_,Value>, _err: MutableHandle<'_,Value>| {
        unsafe { x.to_jsval(realm.deref_mut().raw_cx(), ok) }
    }) as ResolutionMarshalling
}
