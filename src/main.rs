use std::{
    cell::UnsafeCell,
    hint,
    sync::atomic::{self, AtomicBool, AtomicU32, Ordering},
    thread,
};

fn sc_fence_1() {
    if cfg!(feature = "fake-fence-1") {
        // Make sure the compiler doesn't do anything tricky to prove this is really the CPU's
        // fault.
        atomic::compiler_fence(Ordering::SeqCst);
    } else {
        atomic::fence(Ordering::SeqCst);
    }
}

fn sc_fence_2() {
    if cfg!(feature = "fake-fence-2") {
        // Make sure the compiler doesn't do anything tricky to prove this is really the CPU's
        // fault.
        atomic::compiler_fence(Ordering::SeqCst);
    } else {
        atomic::fence(Ordering::SeqCst);
    }
}

struct RawBakeryLock<const N: usize> {
    choosing: [AtomicBool; N],
    ticket: [AtomicU32; N],
}

impl<const N: usize> RawBakeryLock<N> {
    fn new() -> Self {
        #![allow(clippy::declare_interior_mutable_const)]

        const NOT_CHOOSING: AtomicBool = AtomicBool::new(false);
        const NO_TICKET: AtomicU32 = AtomicU32::new(0);

        Self {
            choosing: [NOT_CHOOSING; N],
            ticket: [NO_TICKET; N],
        }
    }

    fn lock(&self, thread: usize) {
        let ticket = loop {
            self.choosing[thread].store(true, Ordering::Relaxed);

            // This fence helps enforce the core invariant of the bakery lock: (intuitively) at any
            // given moment, out of all threads that have currently chosen a ticket, _exactly_ the
            // one with minimal `(ticket[i], i)` is in its critical section. It coordinates with the
            // second SC fence in this function to prevent the following store buffering scenario:
            //
            //  Thread 0:                                          Thread 1:
            //
            //  choosing[0] = true                              |  choosing[1] = true
            //                                                  |  ticket[1] = max(ticket[0], ticket[1]) + 1 // 1
            //  // Store from thread 1 not visible:             |
            //  ticket[0] = max(ticket[0], ticket[1]) + 1 // 1  |
            //  choosing[0] = false                             |
            //  choosing[1] == true                             |
            //                                                  |  choosing[1] = false
            //                                                  |  // Stores from thread 0 not visible:
            //                                                  |  choosing[0] == false
            //                                                  |  ticket[0] == 0
            //  choosing[1] == false                            |  // Critical section...
            //  ticket[0] == 1 // (1, 0) < (1, 1)               |  // Critical section...
            //  // Critical section..                           |  // Critical section...
            //
            // The problem here is that thread 1 doesn't see thread 0's write to `choosing[0]` and
            // incorrectly assumes that it now has the lowest-numbered ticket, while thread 0 has
            // already chosen a ticket of 1 as well and can (correctly) enter its critical section
            // because it has priority over thread 1.
            //
            // More formally, abbreviating `choosing` as `c` and `ticket` as `t`, the problematic
            // scenario is a
            //
            // W(c[0], 1) -po-> R(t[1], 0) -rb-> W(t[1], 1) -po-> R(c[0], 0) -rb-> W(c[0], 1)
            //
            // cycle, so SC fences are necessary somewhere along both `po` edges to forbid it. This
            // fence covers the `W c -> R t` edge, while the one below covers the `R c -> W t` edge.
            sc_fence_1();

            let max_existing = self
                .ticket
                .iter()
                .map(|ticket| ticket.load(Ordering::Relaxed))
                .max()
                .unwrap();

            if let Some(ticket) = max_existing.checked_add(1) {
                // Common case: we have a new ticket larger than all tickets observed.
                break ticket;
            }

            // We've failed to get a ticket now because of overflow - stop choosing now to let
            // currently waiting threads into the bakery and try again.
            self.choosing[thread].store(false, Ordering::Relaxed);

            hint::spin_loop();
        };

        self.ticket[thread].store(ticket, Ordering::Relaxed);

        // This fence serves two distinct purposes:
        // 1. It covers the `R c -> W t` edge of the store buffering scenario discussed above.
        // 2. It synchronizes-with the acquire fence in the loop below to make sure that any
        //    threads observing the write to `choosing` below also observe our new ticket.
        sc_fence_2();

        self.choosing[thread].store(false, Ordering::Relaxed);

        for other in 0..N {
            if other == thread {
                continue;
            }

            while self.choosing[other].load(Ordering::Relaxed) {
                hint::spin_loop();
            }

            // Synchronizes-with the SC fence just before the store to `choosing[other]` to make
            // sure we observe the correct value of `ticket[other]` below.
            atomic::fence(Ordering::Acquire);

            loop {
                let other_ticket = self.ticket[other].load(Ordering::Relaxed);
                if other_ticket == 0 || (ticket, thread) < (other_ticket, other) {
                    break;
                }
                hint::spin_loop();
            }
        }

        // Synchronizes-with the release stores to `ticket` by other threads that have already
        // unlocked (as observed by our reads from `ticket`).
        atomic::fence(Ordering::Acquire);
    }

    fn unlock(&self, thread: usize) {
        // Synchronizes-with the acquire fence at the end of `lock` to establish a proper
        // happens-before relationship with future owners.
        self.ticket[thread].store(0, Ordering::Release);
    }
}

struct UnsafeSyncCell<T>(UnsafeCell<T>);
unsafe impl<T> Sync for UnsafeSyncCell<T> {}

fn main() {
    const NUM_THREADS: usize = 10;

    let lock = RawBakeryLock::<NUM_THREADS>::new();
    let mut num = UnsafeSyncCell(UnsafeCell::new(0));

    thread::scope(|scope| {
        for thread_id in 0..NUM_THREADS {
            let lock = &lock;
            let num = &num;
            scope.spawn(move || {
                println!("thread {thread_id} startup");
                for _ in 0..100000 {
                    lock.lock(thread_id);
                    unsafe {
                        *num.0.get() += 1;
                    }
                    lock.unlock(thread_id);
                }
            });
        }
    });

    println!("{}", num.0.get_mut());
}
