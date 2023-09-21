//! This crate provides a thread-safe, reference counted pointer that uses
//! copy-on-write semantics to allow mutability.
//!
//! I created it because I didn't know about [`Arc::make_mut`][6], which has
//! been available in standard Rust since 2015. I thought about deprecating it
//! in favor of the standard solution, but there are a couple slight advantages
//! that just barely justify it:
//!
//! - `Arcow` is slightly more ergonomic than `Arc::make_mut`. (Phrased as a
//!   disadvantage, it's easier to accidentally mutate through an `Arcow`.)
//! - `Arcow` does not support weak references, which makes it *slightly* more
//!   efficient at runtime.
//!
//! # How
//!
//! Make an `Arcow<T>`. This acts like a cheaply-clonable `T` that can
//! be mutated normally, even if `T` is actually expensive to clone. You can
//! freely send clones of `Arcow<T>` to multiple threads.
//!
//! Note that atomic operations aren't free; this is why `Arcow` doesn't
//! implement [`Copy`][1]. Cloning `Arcow<T>` and sending that to another
//! thread will only be cheaper than cloning `T` if:
//!
//! 1. `T` is big (expensive to clone), **AND**
//! 2. A significant proportion of `Arcow<T>` clones will be used and
//!    consumed without mutation.
//!
//! # Why not
//! 
//! You should consider using [`Cow<T>`][2] if possible. And if you want
//! mutations to propagate to other references to the same underlying `T`,
//! then you don't want copy-on-write at all, you actually want to do something
//! like wrap a [`Mutex<T>`][4] in an [`Arc`][3].
//!
//! # Why
//!
//! Say you're implementing a game server. You keep track of several iterations
//! of game state, potentially a lot of them, to allow for saving, client
//! prediction, and other issues. Most of your game state is trivial, so each
//! iteration just has a separate copy. But your game takes place on a `Map`,
//! and that `Map` is typically around 64KiB. That's not very much in the grand
//! scheme of things, but it quickly adds up, and the overhead of copying the
//! map into every new iteration wastes a lot of CPU time. Your automated tests
//! are particularly affected, as they spend almost all their time copying
//! instead of actually testing your logic.
//!
//! So you wrap the `Map` in an [`Arc`][3]. Now you've replaced a 64KiB copy
//! with a single atomic operation. You pat yourself on the back and proceed.
//!
//! Except that the `Map` can change.
//!
//! In the vast majority of iterations, the `Map` isn't changed. But every so
//! often, somebody will chop down a tree, or build a wall, or pave a road, and
//! when they do, those changes are reflected in the `Map`. If you remove the
//! `Arc`, this is no problem, since each iteration has its own copy of the map
//! to mutate. But now you're copying a whole 64KiB for every iteration, even
//! though less than 1% of iterations will subsequently mutate the map.
//!
//! [`Cow`][2] doesn't work for this case, because there's no long-lived
//! "master copy" of the map that all the other game states can borrow from. No
//! particular iteration can be trusted for that. Old iterations will be pruned
//! as they stop being relevant, and new iterations aren't necessarily going to
//! be kept (e.g. abandoned prediction timelines).
//!
//! Cloning the `Arc<Map>` every time there's a change is a working solution,
//! but what if two players chop down a tree in the same tick? That's a wasted
//! copy. And then there's the case where you're an internal server of a client
//! in a singleplayer mode, and there's only ever one relevant iteration... now
//! you're making a copy of the `Map` every change even though there's never
//! more than one reference!
//!
//! Thus, `Arcow`. It acts very much like a simplified `Arc`, with one
//! additional feature: it can be mutated. A living reference-counted pointer
//! is either unique or shared. If you have a unique `Arcow`, mutation is
//! simple. [`DerefMut`][5] will return a mutable reference to the inner type.
//! As long as that mutable reference is alive, you can't make another clone of
//! the `Arcow`\*, so uniqueness is preserved. If you have a shared `Arcow`,
//! [`DerefMut`][5] will first "split" your `Arcow` into a unique clone of the
//! original shared value, and then you're all set.
//!
//! (\*Not quite accurate. You can clone an `Arcow` that you've currently
//! mutably borrowed, you just can't mutate through that borrow anymore
//! afterwards. See:)
//!
//! ```rust,compile_fail
//! # use arcow::Arcow;
//! // error[E0502]: cannot borrow `a` as immutable because it is also borrowed as mutable
//!    let mut a = Arcow::new(456);
//!    let borrowed = &mut *a;
//! //                      - mutable borrow occurs here
//!    *borrowed = 42;
//!    let b = a.clone();
//! //         ^^^^^^^^^ immutable borrow occurs here
//!    *borrowed = 47;
//! // --------- mutable borrow later used here
//! ```
//!
//! # Legalese
//!
//! Arcow is copyright 2022, 2023 Solra Bizna, and licensed under either of:
//! 
//!  * Apache License, Version 2.0
//!    ([LICENSE-APACHE](LICENSE-APACHE) or
//!    <http://www.apache.org/licenses/LICENSE-2.0>)
//!  * MIT license
//!    ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)
//! 
//! at your option.
//! 
//! Unless you explicitly state otherwise, any contribution intentionally
//! submitted for inclusion in the Arcow crate by you, as defined
//! in the Apache-2.0 license, shall be dual licensed as above, without any
//! additional terms or conditions.
//!
//! [1]: https://doc.rust-lang.org/std/marker/trait.Copy.html
//! [2]: https://doc.rust-lang.org/std/borrow/enum.Cow.html
//! [3]: https://doc.rust-lang.org/std/sync/struct.Arc.html
//! [4]: https://doc.rust-lang.org/std/sync/struct.Mutex.html
//! [5]: https://doc.rust-lang.org/std/ops/trait.DerefMut.html
//! [6]: https://doc.rust-lang.org/std/sync/struct.Arc.html#method.make_mut

use std::{
    fmt::{Debug, Display, Formatter, Result as FmtResult},
    ops::{Deref, DerefMut},
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

struct ArcowInner<T: Clone> {
    refcount: AtomicUsize,
    inner: T,
}

/// Atomically Reference-counted Copy-On-Write shared pointer.
///
/// See the [crate documentation](index.html) for more details.
pub struct Arcow<T: Clone> {
    inner: NonNull<ArcowInner<T>>,
}

impl<T: Debug + Clone> Debug for Arcow<T> {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> FmtResult {
        let inner = unsafe { self.inner.as_ref() };
        write!(fmt, "Arcow/{}{{",
               inner.refcount.load(Ordering::Relaxed))?;
        Debug::fmt(&inner.inner, fmt)?;
        write!(fmt, "}}")?;
        Ok(())
    }
}

impl<T: Display + Clone> Display for Arcow<T> {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> FmtResult {
        let inner = unsafe { self.inner.as_ref() };
        Display::fmt(&inner.inner, fmt)
    }
}

impl<T: Clone> Arcow<T> {
    /// Wrap the given value in a new `Arcow`.
    pub fn new(inner: T) -> Arcow<T> {
        let inner = Box::new(ArcowInner { refcount: AtomicUsize::new(1),
                                          inner });
        Arcow { inner: Box::leak(inner).into() }
    }
    /// Returns the number of references that exist to this same wrapped
    /// object.
    ///
    /// If this returns 1, then the pointer is unique, and mutation is cheap.
    /// More than 1, and it will "split" into a unique clone before mutating.
    /// (This happens automatically, without you having to make any special
    /// effort.)
    pub fn count(myself: &Arcow<T>) -> usize {
        unsafe {
            myself.inner.as_ref().refcount.load(Ordering::Relaxed)
        }
    }
}

impl<T: Clone> Deref for Arcow<T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe {
            &self.inner.as_ref().inner
        }
    }
}

impl<T: Clone> DerefMut for Arcow<T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe {
            if Arcow::count(self) > 1 {
                *self = Arcow::new(self.inner.as_ref().inner.clone());
            }
            &mut self.inner.as_mut().inner
        }
    }
}

impl<T: Clone> Clone for Arcow<T> {
    fn clone(&self) -> Arcow<T> {
        unsafe {
            self.inner.as_ref().refcount.fetch_add(1, Ordering::Acquire);
        }
        Arcow { inner: self.inner }
    }
}

impl<T: Clone> Drop for Arcow<T> {
    fn drop(&mut self) {
        let old_count = unsafe {
            self.inner.as_ref().refcount.fetch_sub(1, Ordering::Release)
        };
        if old_count == 1 {
            unsafe {
                drop(Box::from_raw(self.inner.as_ptr()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        rc::Rc,
        sync::Mutex,
    };
    #[test]
    fn basic() {
        let a = Arcow::new(32);
        let b = a.clone();
        let c = b.clone();
        let mut d = a.clone();
        *d = 64;
        assert_eq!(*a, 32);
        assert_eq!(*b, 32);
        assert_eq!(*c, 32);
        assert_eq!(*d, 64);
        assert_eq!(Arcow::count(&a), 3);
        assert_eq!(Arcow::count(&d), 1);
    }
    /// short for "Unsafe Dropper of Lol".
    /// (it used to be unsafe)
    struct Udl {
        lol: Rc<Mutex<usize>>,
    }
    impl Udl {
        pub fn new(lol: Rc<Mutex<usize>>) -> Udl {
            *lol.lock().unwrap() += 1;
            Udl { lol }
        }
        pub fn mutate(&mut self) {
            // :)
        }
    }
    impl Drop for Udl {
        fn drop(&mut self) {
            let mut locked = self.lol.lock().unwrap();
            if *locked == 0 {
                panic!("It fell below zero!");
            }
            else {
                *locked -= 1;
            }
        }
    }
    impl Clone for Udl {
        fn clone(&self) -> Udl {
            *self.lol.lock().unwrap() += 1;
            Udl { lol: self.lol.clone() }
        }
    }
    #[test]
    fn dropping() {
        let count = Rc::new(Mutex::new(0));
        println!("{} A: exist!", *count.lock().unwrap());
        let a = Arcow::new(Udl::new(count.clone()));
        assert_eq!(*count.lock().unwrap(), 1);
        println!("{} B: become A!", *count.lock().unwrap());
        let b = a.clone();
        assert_eq!(*count.lock().unwrap(), 1);
        println!("{} C: become B!", *count.lock().unwrap());
        let c = b.clone();
        assert_eq!(*count.lock().unwrap(), 1);
        println!("{} D: become A!", *count.lock().unwrap());
        let mut d = a.clone();
        assert_eq!(*count.lock().unwrap(), 1);
        println!("{} E: become A!", *count.lock().unwrap());
        let e = a.clone();
        assert_eq!(*count.lock().unwrap(), 1);
        println!("{} E: drop", *count.lock().unwrap());
        drop(e);
        assert_eq!(*count.lock().unwrap(), 1);
        println!("{} D: mutate!", *count.lock().unwrap());
        d.mutate();
        assert_eq!(*count.lock().unwrap(), 2);
        println!("{} B: drop", *count.lock().unwrap());
        drop(b);
        assert_eq!(*count.lock().unwrap(), 2);
        println!("{} C: drop", *count.lock().unwrap());
        drop(c);
        assert_eq!(*count.lock().unwrap(), 2);
        println!("{} D: drop", *count.lock().unwrap());
        drop(d);
        assert_eq!(*count.lock().unwrap(), 1);
        println!("{} A: drop", *count.lock().unwrap());
        drop(a);
        assert_eq!(*count.lock().unwrap(), 0);
    }
}
