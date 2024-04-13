# Bakery

A simple example of what can go wrong when implementing [Lamport's bakery algorithm](https://en.wikipedia.org/wiki/Lamport%27s_bakery_algorithm) on modern hardware.

Under the C11 memory model (and all modern hardware models) a correct implementation of this algorithm requires two sequentially-consistent fences during the `lock` operation. Replacing either of these fences with a compiler-only fence prevents it from guaranteeing mutual exclusion.

This program uses the bakery algorithm to protect a shared counter across a number of threads to demonstrate the problem in practice.

On my Alder Lake laptop, the program consistently counts to 1000000 with both fences intact, but things can get wacky when removing either of them. For example:

```bash
# leave only second fence
$ cargo run --release -F fake-fence-1
    Finished release [optimized] target(s) in 0.00s
     Running `target/release/bakery`
thread 0 startup
thread 2 startup
thread 1 startup
thread 4 startup
thread 3 startup
thread 9 startup
thread 5 startup
thread 7 startup
thread 8 startup
thread 6 startup
999993

# leave only first fence
cargo run --release -F fake-fence-2
    Finished release [optimized] target(s) in 0.00s
     Running `target/release/bakery`
thread 1 startup
thread 2 startup
thread 0 startup
thread 3 startup
thread 5 startup
thread 4 startup
thread 8 startup
thread 9 startup
thread 7 startup
thread 6 startup
999920
```
