use std::{
    pin::{Pin},
    future::{Future,poll_fn},
    task::{Context,Poll},
    ptr::{NonNull},
    rc::{Rc},
    marker::{PhantomData},
    cell::{RefCell,LazyCell},
    collections::{BTreeMap},
    sync::atomic::{AtomicU64,Ordering},
};
use tokio::{
    runtime::{Handle as TokioHandle},
};
use futures_util::{
    stream::futures_unordered::FuturesUnordered,
    stream::{Stream},
};
use mozjs::{rooted};
use mozjs::{
    realm::{AutoRealm},
    context::{JSContext},
    jsapi::{Heap,JSObject,Value},
    jsval::{UndefinedValue},
    gc::{Handle,MutableHandle},
};

use super::incumbent_stack::{push_incumbent_stack};


thread_local! {
    static IDFK: AtomicU64 = AtomicU64::new(0);
    static FAKE_PENDING_PROMISES: LazyCell<RefCell<BTreeMap<u64,InternalPromise>>> = LazyCell::new(|| RefCell::new(BTreeMap::new()));
    /// Ensures our promise handling is non-reentrant
    static PENDING: LazyCell<RefCell<FuturesUnordered<Pin<Box<dyn Future<Output=(u64,ResolutionMarshalling)> + 'static>>>>> = LazyCell::new(|| RefCell::new(FuturesUnordered::new()));
}

pub fn why_is_promise_id_segfaulting() -> u64 {
    IDFK.with(|i| i.fetch_add(1,Ordering::Relaxed))
}

pub(crate) struct InternalPromise {
    pub(crate) promise: Box<Heap<*mut JSObject>>,
    pub(crate) global: Rc<Box<Heap<*mut JSObject>>>,
}

pub(crate) fn push_internal_promise(id: u64, promise: Box<Heap<*mut JSObject>>, global: Rc<Box<Heap<*mut JSObject>>>) {
    FAKE_PENDING_PROMISES.with(|f| f.borrow_mut().insert(id, InternalPromise { promise, global }));
}
pub fn futures_empty() -> bool {
    PENDING.with(|p| p.borrow().is_empty())
}

pub fn poll_futures() -> Vec<(u64,ResolutionMarshalling)> {

    // types get really funky so this is in its own place
    fn poll_the_stream(pool: &mut FuturesUnordered<Pin<Box<dyn Future<Output=(u64,ResolutionMarshalling)> + 'static>>>) -> Vec<(u64,ResolutionMarshalling)> {
        let handle = TokioHandle::current();
        let mut tasks = Vec::new();
        handle.block_on(poll_fn(|ctx: &mut Context<'_>| -> Poll<()> {
            loop {
                if pool.is_empty() {
                    return Poll::Ready(());
                }
                // we cannot await indefinately as this is a singled threaded run
                //
                // so when we see a `Poll::Pending` we assume our Context/Waker
                // is setup right and return what ever we've captured so far.
                //
                let pin: Pin<&mut FuturesUnordered<Pin<Box<dyn Future<Output=(u64,ResolutionMarshalling)> + 'static>>>> = Pin::new(pool);
                match pin.poll_next(ctx) {
                    Poll::Pending => return Poll::Ready(()),
                    Poll::Ready(None) => return Poll::Ready(()),
                    Poll::Ready(Some((key,arg))) => {
                        tasks.push((key,arg));
                        continue;
                    }
                };
            }
        }));
        tasks
    }


    PENDING.with(|p| poll_the_stream(&mut p.borrow_mut()))
}

pub fn setup_to_resolve(ctx: &mut JSContext, id: u64, lambda: ResolutionMarshalling) {
    let data: InternalPromise = FAKE_PENDING_PROMISES.with(|f| f.borrow_mut().remove(&id).unwrap());
    push_incumbent_stack(data.global.clone());

    rooted!(in(unsafe { ctx.raw_cx() }) let global = data.global.get());
    rooted!(in(unsafe { ctx.raw_cx() }) let promise = data.promise.get());
    rooted!(in(unsafe { ctx.raw_cx() }) let mut ok = UndefinedValue());
    rooted!(in(unsafe { ctx.raw_cx() }) let mut err = UndefinedValue());

    let mut realm = AutoRealm::new(ctx, NonNull::new(global.handle().get()).unwrap());
        let (global, realm) = realm.global_and_reborrow();
    (lambda)(realm, promise.handle(), global, ok.handle_mut(), err.handle_mut());
    if !err.is_undefined() {
        unsafe { mozjs::rust::wrappers2::RejectPromise(realm, promise.handle(), err.handle()) };
    } else {
        unsafe { mozjs::rust::wrappers2::ResolvePromise(realm, promise.handle(), ok.handle()) };
    }
}


/// Bridges between the JS & Async runtime.
///
/// This type is extremely deligate as we have to balance access
/// to a number of global thread local resources and **never**
/// alias mutable access. 
///
/// # General overview
///
/// - Interally Bridge stores a future which references the global runtime
///     - This future runs `(u64,R)`
///     - `u64` is the PromiseID, this tells us what promise we will resolve
///     - `R` is task specific data.
/// - When the resolved `bridge` callback is invoked.
///     - `bridge` is a partial function
///     - `bridge` exists to "hide" the runtime from Threadlocal structures
///     - The `FnOnce` returned by `bridge` is reponse for creating `ResolvablePromise`
///       that will mange resolving the underlying promise.
///     - Neither `bridge` nor the `FnOnce` it returns should attempt to
///       resolve the promise early. This will result in a segmentation fault.
///
/// # Important Notes
///
/// The internal bridge callback executes on the same thread
/// as the java script engine, but without access to the context
/// and/or javascript autorealm of the final future. 
///
/// It is therefore expected to prepare a `FnOnce` which **will**
/// have access to the correct javascript context, autorealm, and
/// globals so the result maybe marshaled into JS native format.
///
/// Critically while `FnOnce` is ran the JS Engine has not yielded
/// for the async runtime to trigger tasks. Therefore the `FnOnce`
/// **cannot under any circumstances** attempt to resolve the promise
/// itself.
///
/// The result of the `FnOnce` is `ResolvablePromise` which will
/// hande the Resolution/Rejection of the promise, but only once
/// the underlying engine has yielded to `runJobs`.
///
pub struct Bridge<R> {
    pub internal: Pin<Box<dyn Future<Output=(u64,R)> + Send + 'static>>,
    pub bridge: fn(R) -> ResolutionMarshalling,
    // ensure this type cannot be sent between threads
    _marker: PhantomData<Rc<()>>,
}
impl<R> Bridge<R>
where
    R: Send + 'static,
{
    pub(crate) fn new<F>(future: F, bridge: fn(R) -> ResolutionMarshalling)
    where
        F: Future<Output=(u64,R)> + Send + 'static,
    {
        let b = Bridge {
            internal: Box::pin(future),
            bridge,
            _marker: PhantomData,
        };
        PENDING.with(|p| p.borrow().push(Box::pin(b)));
    }
}

/// Converts the `R` returned by `Bridge` into a resolvable promise
pub type ResolutionMarshalling = Box<dyn 'static + for<'a> FnOnce(&mut AutoRealm,Handle<'a,*mut JSObject>,Handle<'a,*mut JSObject>,MutableHandle<'a,Value>,MutableHandle<'a,Value>)>;

impl<R> Future for Bridge<R>
where
    R: Send + 'static,
{
    type Output = (u64,ResolutionMarshalling);
    fn poll(self: Pin<&mut Self>, ctx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this: &mut Self = self.get_mut();
        let (id, result): (u64, R) = match this.internal.as_mut().poll(ctx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(tup) => tup,
        };
        let lambda = (this.bridge)(result);
        Poll::Ready((id,lambda))
    }
}
