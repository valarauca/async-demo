use std::{
    cell::{LazyCell,RefCell},
    rc::{Rc},
};
use mozjs::jsapi::{Heap,JSObject};


thread_local! {
    static INCUMBENT_STACK: LazyCell<RefCell<Vec<Rc<Box<Heap<*mut JSObject>>>>>> = LazyCell::new(|| RefCell::new(Vec::new()));
}

/// Pushes an item into the incumbent stack
pub(crate) fn push_incumbent_stack<T>(item: T)
where
    Rc<Box<Heap<*mut JSObject>>>: From<T>,
{
    INCUMBENT_STACK.with(|inner| {
        inner.borrow_mut().push(Rc::from(item));
    })
}

/// Pops an item from the incumbent stack
pub(crate) fn pop_incumbent_stack() -> Option<Rc<Box<Heap<*mut JSObject>>>> {
    INCUMBENT_STACK.with(|inner| {
        inner.borrow_mut().pop()
    })
}

/// Peak at what is on the top of our stack
pub(crate) fn peek_incumbent_stack() -> Option<Rc<Box<Heap<*mut JSObject>>>> {
    INCUMBENT_STACK.with(|inner| {
        inner.borrow_mut().last().map(|x: &Rc<Box<Heap<*mut JSObject>>>| x.clone())
    })
}

