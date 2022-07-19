# arcow

This crate provides a thread-safe, reference counted pointer that uses
copy-on-write semantics to allow mutability.

## How

Make an `Arcow<T>`. This acts like a cheaply-clonable `T` that can
be mutated normally, even if `T` is actually expensive to clone. You can
freely send clones of `Arcow<T>` to multiple threads.

Note that atomic operations aren't free; this is why `Arcow` doesn't
implement [`Copy`][1]. Cloning `Arcow<T>` and sending that to another
thread will only be cheaper than cloning `T` if:

1. `T` is big (expensive to clone), **AND**
2. A significant proportion of `Arcow<T>` clones will be used and
   consumed without mutation.

## Why not

You should consider using [`Cow<T>`][2] if possible. And if you want
mutations to propagate to other references to the same underlying `T`,
then you don't want copy-on-write at all, you actually want to do something
like wrap a [`Mutex<T>`][4] in an [`Arc`][3].

## Why

Say you're implementing a game server. You keep track of several iterations
of game state, potentially a lot of them, to allow for saving, client
prediction, and other issues. Most of your game state is trivial, so each
iteration just has a separate copy. But your game takes place on a `Map`,
and that `Map` is typically around 64KiB. That's not very much in the grand
scheme of things, but it quickly adds up, and the overhead of copying the
map into every new iteration wastes a lot of CPU time. Your automated tests
are particularly affected, as they spend almost all their time copying
instead of actually testing your logic.

So you wrap the `Map` in an [`Arc`][3]. Now you've replaced a 64KiB copy
with a single atomic operation. You pat yourself on the back and proceed.

Except that the `Map` can change.

In the vast majority of iterations, the `Map` isn't changed. But every so
often, somebody will chop down a tree, or build a wall, or pave a road, and
when they do, those changes are reflected in the `Map`. If you remove the
`Arc`, this is no problem, since each iteration has its own copy of the map
to mutate. But now you're copying a whole 64KiB for every iteration, even
though less than 1% of iterations will subsequently mutate the map.

[`Cow`][2] doesn't work for this case, because there's no long-lived
"master copy" of the map that all the other game states can borrow from. No
particular iteration can be trusted for that. Old iterations will be pruned
as they stop being relevant, and new iterations aren't necessarily going to
be kept (e.g. abandoned prediction timelines).

Cloning the `Arc<Map>` every time there's a change is a working solution,
but what if two players chop down a tree in the same tick? That's a wasted
copy. And then there's the case where you're an internal server of a client
in a singleplayer mode, and there's only ever one relevant iteration... now
you're making a copy of the `Map` every change even though there's never
more than one reference!

Thus, `Arcow`. It acts very much like a simplified `Arc`, with one
additional feature: it can be mutated. A living reference-counted pointer
is either unique or shared. If you have a unique `Arcow`, mutation is
simple. [`DerefMut`][5] will return a mutable reference to the inner type.
As long as that mutable reference is alive, you can't make another clone of
the `Arcow`\*, so uniqueness is preserved. If you have a shared `Arcow`,
[`DerefMut`][5] will first "split" your `Arcow` into a unique clone of the
original shared value, and then you're all set.

(\*Not quite accurate. You can clone an `Arcow` that you've currently
mutably borrowed, you just can't mutate through that borrow anymore
afterwards. See:)

```rust,compile_fail
# use arcow::Arcow;
// error[E0502]: cannot borrow `a` as immutable because it is also borrowed as mutable
   let mut a = Arcow::new(456);
   let borrowed = &mut *a;
//                      - mutable borrow occurs here
   *borrowed = 42;
   let b = a.clone();
//         ^^^^^^^^^ immutable borrow occurs here
   *borrowed = 47;
// --------- mutable borrow later used here
```

## Legalese

Arcow is copyright 2022, Solra Bizna, and licensed under either of:

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or
   <http://www.apache.org/licenses/LICENSE-2.0>)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the Arcow crate by you, as defined
in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.

[1]: https://doc.rust-lang.org/std/marker/trait.Copy.html
[2]: https://doc.rust-lang.org/std/borrow/enum.Cow.html
[3]: https://doc.rust-lang.org/std/sync/struct.Arc.html
[4]: https://doc.rust-lang.org/std/sync/struct.Mutex.html
[5]: https://doc.rust-lang.org/std/ops/trait.DerefMut.html
