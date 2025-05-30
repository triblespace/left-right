//! A concurrency primitive for high concurrency reads over a single-writer data structure.
//!
//! The primitive keeps two copies of the backing data structure, one that is accessed by readers,
//! and one that is accessed by the (single) writer. This enables all reads to proceed in parallel
//! with minimal coordination, and shifts the coordination overhead to the writer. In the absence
//! of writes, reads scale linearly with the number of cores.
//!
//! When the writer wishes to expose new changes to the datastructure (see
//! [`WriteHandle::publish`]), it "flips" the two copies so that subsequent reads go to the old
//! "write side", and future writers go to the old "read side". This process does cause two cache
//! line invalidations for the readers, but does not stop them from making progress (i.e., reads
//! are wait-free).
//!
//! In order to keep both copies up to date, left-right keeps an operational log ("oplog") of all
//! the modifications to the data structure, which it uses to bring the old read data up to date
//! with the latest writes on a flip. Since there are two copies of the data, each oplog entry is
//! applied twice: once to the write copy and again to the (stale) read copy.
//!
//! # Trade-offs
//!
//! Few concurrency wins come for free, and this one is no exception. The drawbacks of this
//! primitive are:
//!
//!  - **Increased memory use**: since we keep two copies of the backing data structure, we are
//!  effectively doubling the memory use of the underlying data. With some clever de-duplication,
//!  this cost can be ameliorated to some degree, but it's something to be aware of. Furthermore,
//!  if writers only call `publish` infrequently despite adding many writes to the operational log,
//!  the operational log itself may grow quite large, which adds additional overhead.
//!  - **Deterministic operations**: as the entries in the operational log are applied twice, once
//!  to each copy of the data, it is essential that the operations are deterministic. If they are
//!  not, the two copies will no longer mirror one another, and will continue to diverge over time.
//!  - **Single writer**: left-right only supports a single writer. To have multiple writers, you
//!  need to ensure exclusive access to the [`WriteHandle`] through something like a
//!  [`Mutex`](std::sync::Mutex).
//!  - **Slow writes**: Writes through left-right are slower than they would be directly against
//!  the backing datastructure. This is both because they have to go through the operational log,
//!  and because they must each be applied twice.
//!
//! # How does it work?
//!
//! Take a look at [this YouTube video](https://www.youtube.com/watch?v=eLNAMEoKAAc) which goes
//! through the basic concurrency algorithm, as well as the initial development of this library.
//! Alternatively, there's a shorter (but also less complete) description in [this
//! talk](https://www.youtube.com/watch?v=s19G6n0UjsM&t=1994).
//!
//! At a glance, left-right is implemented using two regular `T`s,
//! an auxiliary type `A` which is only used during writes, an operational log, epoch
//! counting, and some pointer magic. There is a single pointer through which all readers go. It
//! points to a `T` that the readers access in order to read data. Every time a read has accessed
//! the pointer, they increment a local epoch counter, and they update it again when they have
//! finished the read. When a write occurs, the writer updates the other `T` (for which there are
//! no readers), and also stores a copy of the change in a log. When [`WriteHandle::publish`] is
//! called, the writer, atomically swaps the reader pointer to point to the other `T`. It then
//! waits for the epochs of all current readers to change, and then replays the operational log to
//! bring the stale copy up to date.
//!
//! The design resembles this [left-right concurrency
//! scheme](https://hal.archives-ouvertes.fr/hal-01207881/document) from 2015, though I am not
//! aware of any follow-up to that work.
//!
//! # How do I use it?
//!
//! If you just want a data structure for fast reads, you likely want to use a crate that _uses_
//! this crate, like [`evmap`](https://docs.rs/evmap/). If you want to develop such a crate
//! yourself, here's what you do:
//!
//! ```rust
//! use reft_light::{Apply, ReadHandle, WriteHandle};
//!
//! // First, define an operational log type.
//! // For most real-world use-cases, this will be an `enum`, but we'll keep it simple:
//! struct CounterAddOp(i32);
//!
//! // Then, implement the `Apply` trait for that type,
//! // and provide the datastructure types it operates over as generic arguments.
//! // You can read this as "`CounterAddOp` can apply changes of types `i32` and `()`".
//! impl Apply<i32, ()> for CounterAddOp {
//!     // See the documentation of `Apply::apply_first`.
//!     //
//!     // Essentially, this is where you define what applying
//!     // the oplog type to the datastructure does.
//!     fn apply_first(&mut self, first: &mut i32, _: &i32, _: &mut ()) {
//!         *first += self.0;
//!     }
//!
//!     // See the documentation of `Apply::apply_second`.
//!     //
//!     // This may or may not be the same as `apply_first`,
//!     // depending on whether or not you de-duplicate values
//!     // across the two copies of your data structure.
//!     fn apply_second(self, _: &i32, second: &mut i32, _: &mut ()) {
//!         *second += self.0;
//!     }
//! }
//!
//! // Now, you can construct a new left-right over an instance of your data structure.
//! // This will give you a `WriteHandle` that accepts writes in the form of oplog entries,
//! // which can be converted into a (cloneable) `ReadHandle` that gives you `&` access to the data structure.
//! let write = reft_light::new::<CounterAddOp, i32, ()>(0, ());
//! let read = write.clone();
//!
//! // You will likely want to embed these handles in your own types so that you can
//! // provide more ergonomic methods for performing operations on your type.
//! struct Counter(WriteHandle<CounterAddOp, i32, ()>);
//! impl Counter {
//!     // The methods on you write handle type will likely all just add to the operational log.
//!     pub fn add(&mut self, i: i32) {
//!         self.0.append(CounterAddOp(i));
//!     }
//!
//!     // You should also provide a method for exposing the results of any pending operations.
//!     //
//!     // Until this is called, any writes made since the last call to `publish` will not be
//!     // visible to readers. See `WriteHandle::publish` for more details. Make sure to call
//!     // this out in _your_ documentation as well, so that your users will be aware of this
//!     // "weird" behavior.
//!     pub fn publish(&mut self) {
//!         self.0.publish();
//!     }
//! }
//!
//! // Similarly, for reads:
//! #[derive(Clone)]
//! struct CountReader(ReadHandle<i32>);
//! impl CountReader {
//!     pub fn get(&self) -> i32 {
//!         // The `ReadHandle` itself does not allow you to access the underlying data.
//!         // Instead, you must first "enter" the data structure. This is similar to
//!         // taking a `Mutex`, except that no lock is actually taken. When you enter,
//!         // you are given back a guard, which gives you shared access (through the
//!         // `Deref` trait) to the "read copy" of the data structure.
//!         //
//!         // Note that `enter` may yield `None`, which implies that the `WriteHandle`
//!         // was dropped, and took the backing data down with it.
//!         //
//!         // Note also that for as long as the guard lives, a writer that tries to
//!         // call `WriteHandle::publish` will be blocked from making progress.
//!         self.0.enter().map(|guard| *guard).unwrap_or(0)
//!     }
//! }
//!
//! // These wrapper types are likely what you'll give out to your consumers.
//! let (mut w, r) = (Counter(write), CountReader(read));
//!
//! // They can then use the type fairly ergonomically:
//! assert_eq!(r.get(), 0);
//! w.add(1);
//! // no call to publish, so read side remains the same:
//! assert_eq!(r.get(), 0);
//! w.publish();
//! assert_eq!(r.get(), 1);
//! drop(w);
//! // writer dropped data, so reads yield fallback value:
//! assert_eq!(r.get(), 0);
//! ```
//!
//! One additional noteworthy detail: much like with `Mutex`, `RwLock`, and `RefCell` from the
//! standard library, the values you dereference out of a `ReadGuard` are tied to the lifetime of
//! that `ReadGuard`. This can make it awkward to write ergonomic methods on the read handle that
//! return references into the underlying data, and may tempt you to clone the data out or take a
//! closure instead. Instead, consider using [`ReadGuard::map`] and [`ReadGuard::try_map`], which
//! (like `RefCell`'s [`Ref::map`](std::cell::Ref::map)) allow you to provide a guarded reference
//! deeper into your data structure.
#![warn(
    missing_docs,
    rust_2018_idioms,
    missing_debug_implementations,
    broken_intra_doc_links
)]
#![allow(clippy::type_complexity)]

mod sync;

use crate::sync::{Arc, AtomicUsize, Mutex};

type Epochs = Arc<Mutex<slab::Slab<Arc<AtomicUsize>>>>;

mod write;
pub use crate::write::WriteHandle;

mod read;
pub use crate::read::{ReadGuard, ReadHandle, ReadHandleFactory};

/// Types that can incorporate operations of type `O`.
///
/// This trait allows `left-right` to keep the two copies of the underlying data structure (see the
/// [crate-level documentation](crate)) the same over time. Each write operation to the data
/// structure is logged as an operation of type `O` in an _operational log_ (oplog), and is applied
/// once to each copy of the data.
///
/// Implementations should ensure that the application of each operation is deterministic. That is, if
/// two instances of the type `T` are initially equal, and the same operation is applied to both of them,
/// they should remain equal afterwards. If this is not the case, the two copies will drift apart
/// over time, and hold different values.
///
/// The trait provides separate methods for the first and second application of each operation. For many
/// implementations, these will be the same (which is why `apply_second` defaults to calling
/// `apply_first`), but not all. In particular, some implementations may need to modify the operation to
/// ensure deterministic results when it is applied to the second copy.
pub trait Apply<T, A>: Sized {
    /// Apply `O` to the first of the two copies.
    ///
    /// `other` is a reference to the other copy of the data, which has seen all operations up
    /// until the previous call to [`WriteHandle::publish`]. That is, `other` is one "publish
    /// cycle" behind.
    fn apply_first(&mut self, first: &mut T, second: &T, auxiliary: &mut A);

    /// Apply `O` to the second of the two copies.
    ///
    /// `other` is a reference to the other copy of the data, which has seen all operations up to
    /// the call to [`WriteHandle::publish`] that initially exposed this `O`. That is, `other` is
    /// one "publish cycle" ahead.
    ///
    /// Note that this method should modify the underlying data in _exactly_ the same way as
    /// `O` modified `other`, otherwise the two copies will drift apart. Be particularly mindful of
    /// non-deterministic implementations of traits that are often assumed to be deterministic
    /// (like `Eq` and `Hash`), and of "hidden states" that subtly affect results like the
    /// `RandomState` of a `HashMap` which can change iteration order.
    ///
    /// Defaults to calling `apply_first`.
    fn apply_second(mut self, first: &T, second: &mut T, auxiliary: &mut A) {
        Self::apply_first(&mut self, second, first, auxiliary);
    }
}

/// Construct a new write handle from an initial swapping value and an auxiliary value.
///
/// The swapping type must implement `Clone` so we can construct the second copy from the first.
pub fn new<O, T, A>(init: T, auxiliary: A) -> WriteHandle<O, T, A>
where
    O: Apply<T, A>,
    T: Clone,
{
    let epochs = Default::default();

    let r = ReadHandle::new(init.clone(), Arc::clone(&epochs));
    let w = WriteHandle::new(init, epochs, r, auxiliary);
    w
}
