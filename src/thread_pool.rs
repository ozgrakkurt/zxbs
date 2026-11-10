use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use std::thread;
use std::thread::sleep;
use std::time::Duration;

type Job = Box<dyn FnOnce() + Send + 'static>;

pub struct ThreadPool {
    workers: Vec<thread::JoinHandle<()>>,
    senders: Vec<mpsc::Sender<Job>>,
    next: AtomicUsize,
}

impl ThreadPool {
    pub fn new(num_threads: usize) -> Self {
        assert!(num_threads > 0);
        let mut workers = Vec::with_capacity(num_threads);
        let mut senders = Vec::with_capacity(num_threads);

        for _ in 0..num_threads {
            let (tx, rx) = mpsc::channel::<Job>();
            senders.push(tx);
            workers.push(thread::spawn(move || {
                while let Ok(job) = rx.recv() {
                    job();
                }
            }));
        }

        ThreadPool {
            workers,
            senders,
            next: AtomicUsize::new(0),
        }
    }

    pub fn spawn<F, T>(&self, f: F) -> TaskHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = mpsc::channel::<T>();
        let job: Job = Box::new(move || {
            let _ = tx.send(f());
        });

        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.senders.len();
        self.senders[idx].send(job).expect("worker thread exited");

        TaskHandle { rx }
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        // Drop every Sender → each worker's recv() returns Err → thread exits.
        self.senders.clear();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

pub struct TaskHandle<T> {
    rx: mpsc::Receiver<T>,
}

impl<T> TaskHandle<T> {
    pub fn join(self) -> T {
        self.rx
            .recv()
            .expect("worker panicked before sending a result")
    }
}

// We need to have this abstraction in order to be able to call swap_remove on handles and just overall
// have better control over the underlying implementation.
//
// Also we need the code to be here in order for it to access TaskHandle.rx to call try_recv anyway.
pub struct JoinSet<T> {
    handles: Vec<TaskHandle<T>>,
}

impl<T> JoinSet<T> {
    pub fn new() -> Self {
        Self {
            handles: Vec::new(),
        }
    }

    pub fn spawn<F>(&mut self, thread_pool: &ThreadPool, f: F)
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let handle = thread_pool.spawn(f);
        self.handles.push(handle);
    }

    pub fn len(&self) -> usize {
        self.handles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn poll(&mut self) -> Option<(usize, T)> {
        for (i, handle) in self.handles.iter().enumerate() {
            match handle.rx.try_recv() {
                Ok(v) => {
                    return Some((i, v));
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    unreachable!();
                }
            }
        }

        None
    }

    pub fn join_next(&mut self) -> Option<T> {
        if self.handles.is_empty() {
            return None;
        }

        loop {
            match self.poll() {
                Some((idx, val)) => {
                    self.handles.swap_remove(idx);
                    return Some(val);
                }
                None => {
                    // Hopefully this should keep CPU hogging at a tolerable level.
                    //
                    // Can wait a bit before calling sleep so we poll for a couple iterations before going to sleep if
                    // latency is actually bad, but this should be fine.
                    sleep(Duration::from_nanos(1));
                }
            }
        }
    }

    pub fn join_all(&mut self) -> Vec<T> {
        let mut out = Vec::with_capacity(self.len());

        while let Some(v) = self.join_next() {
            out.push(v);
        }

        out
    }
}
