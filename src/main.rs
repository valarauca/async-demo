use std::{
    ptr,
    ptr::NonNull,
};
use mozjs::{
    rooted,
    rust::{
        JSEngine, Runtime, SIMPLE_GLOBAL_CLASS,
        RealmOptions,
        wrappers2::{
            InitRealmStandardClasses,
            JS_DefineFunction,
        },
    },
    glue::CreateJobQueue,
    jsapi::{
        SetJobQueue, SetPromiseRejectionTrackerCallback,
        OnNewGlobalHookOption, JS_NewGlobalObject,
        Value, CallArgs, Heap,
    },
    jsval::UndefinedValue,
    realm::AutoRealm,
    panic::wrap_panic,
};
use tracing_subscriber::FmtSubscriber;
use tracing::{Level,info};
mod runtime;
mod future_callback;
use self::{
    runtime::callback::JOB_QUEUE_TRAPS,
    runtime::incumbent_stack::{enter_incumbent_stack},
    future_callback::tokio_sleep_ms,
};

fn main() {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::WARN)
        .with_writer(std::io::stderr)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .unwrap();

    info!("logger init");

    let tokio_rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = tokio_rt.enter();
    let engine = JSEngine::init().unwrap();
    let mut runtime = Runtime::new(engine.handle());
    let context = runtime.cx();
    unsafe {
        // setup using a hacky thread local queue
        // no debug interupts
        //
        // this has to occur before any globals
        let job_queue = CreateJobQueue(
            &JOB_QUEUE_TRAPS,
            ptr::null(),
            ptr::null_mut(),
        );
        SetJobQueue(context.raw_cx(), job_queue);

        // don't setup a promis rejection tracker
        // because I know fuck all about java script
        SetPromiseRejectionTrackerCallback(
            context.raw_cx(),
            None,
            ptr::null_mut(),
        );
    }
    let h_option = OnNewGlobalHookOption::FireOnNewGlobalHook;
    let c_option = RealmOptions::default();

    for realm_id in 1..=10 {
        rooted!(in(unsafe { context.raw_cx() }) let global = unsafe {
            JS_NewGlobalObject(
                context.raw_cx(),
                &SIMPLE_GLOBAL_CLASS,
                ptr::null_mut(),
                h_option,
                &*c_option,
            )
        });

        enter_incumbent_stack(context, global.handle(), |realm,global_obj| {
        //let mut realm = AutoRealm::new_from_handle(context, global.handle());
        //let (global_obj, realm) = realm.global_and_reborrow();
        //push_incumbent_stack(Heap::boxed(global_obj.get()));
        
            unsafe {
                InitRealmStandardClasses(realm);
                JS_DefineFunction(realm,global_obj,c"print_stuff".as_ptr(),Some(print_stuff),1,0,);
                JS_DefineFunction(realm,global_obj,c"sleep_ms".as_ptr(),Some(tokio_sleep_ms),1,0,);
            }

            let script = format!(r#"
                let callCount = 0;
                let myId = {};
                
                async function doWork() {{
                    for (let i = 0; i < 10; i++) {{
                        // Random sleep between 25 and 2000 ms
                        let sleepDuration = Math.floor(Math.random() * (2000 - 25 + 1)) + 25;
                        let slept = await sleep_ms(sleepDuration);
                        callCount++;
                        let text = `promise id: '${{myId}}' call count: '${{callCount}}' I slept for '${{slept}}' ms`;
                        print_stuff(text);
                    }}
                }}
                
                doWork();
            "#, realm_id);

            rooted!(&in(realm) let mut rval = UndefinedValue());
            let options = mozjs::rust::CompileOptionsWrapper::new(realm, &format!("realm{}.js", realm_id), 0);
            mozjs::rust::evaluate_script(realm, global_obj, &script, rval.handle_mut(), options)
            .unwrap_or_else(|e| panic!("Failed to evaluate realm {}, {:?}", realm_id, e));
        })
    }

    crate::runtime::checkpoint::runtime_checkpoint(context);
}


unsafe extern "C" fn print_stuff(
    ctx: *mut mozjs::context::RawJSContext,
    argc: u32,
    vp: *mut Value,
) -> bool {
    let mut is_okay = true;
    wrap_panic(&mut || {
        let args = unsafe { CallArgs::from_vp(vp, argc) };
        if args.argc_ < 1 {
            is_okay = false;
            return;
        }
        let arg = args.get(0).to_string();
        let s = unsafe { mozjs::conversions::jsstr_to_string(ctx, NonNull::new(arg).unwrap()) };
        println!("{}", s);
        args.rval().set(UndefinedValue());
    });
    true
}
