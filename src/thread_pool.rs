use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use std::thread;
use std::thread::sleep;
use std::time::{Duration, Instant};

type Job = Box<dyn FnOnce() + Send + 'static>;

pub struct ThreadPool {
    senders: Vec<Vec<mpsc::Sender<Job>>>,
    next: Vec<AtomicUsize>,
}

impl ThreadPool {
    pub fn new(num_threads: usize, num_layers: usize) -> Self {
        assert!(num_threads > 0);
        let mut senders = Vec::with_capacity(num_layers);

        for _ in 0..num_layers {
            let mut s = Vec::with_capacity(num_threads);

            for _ in 0..num_threads {
                let (tx, rx) = mpsc::channel::<Job>();
                s.push(tx);
                thread::spawn(move || {
                    while let Ok(job) = rx.recv() {
                        job();
                    }
                });
            }

            senders.push(s);
        }

        let mut next = Vec::with_capacity(num_layers);

        for _ in 0..num_layers {
            next.push(AtomicUsize::new(0));
        }

        ThreadPool { senders, next }
    }

    pub fn spawn<F, T>(&self, layer: usize, f: F) -> TaskHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = mpsc::channel::<T>();
        let job: Job = Box::new(move || {
            let _ = tx.send(f());
        });

        let idx = self.next[layer].fetch_add(1, Ordering::Relaxed) % self.senders.len();
        self.senders[layer][idx]
            .send(job)
            .expect("worker thread exited");

        TaskHandle { rx }
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

#[derive(Clone, Copy)]
pub struct ThreadPoolHandle<'a> {
    pool: &'a ThreadPool,
    layer: usize,
}

impl<'a> ThreadPoolHandle<'a> {
    pub fn new(thread_pool: &'a ThreadPool) -> Self {
        Self {
            pool: thread_pool,
            layer: 0,
        }
    }

    pub fn next_layer(&self) -> Self {
        Self {
            pool: self.pool,
            layer: self.layer + 1,
        }
    }

    pub fn spawn<F, T>(&self, f: F) -> TaskHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.pool.spawn(self.layer, f)
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

    pub fn spawn<F>(&mut self, thread_pool_handle: ThreadPoolHandle, f: F)
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let handle = thread_pool_handle.spawn(f);
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

        let mut polling_since = Instant::now();

        loop {
            match self.poll() {
                Some((idx, val)) => {
                    self.handles.swap_remove(idx);
                    return Some(val);
                }
                None => {
                    if polling_since.elapsed().as_millis() > 2 {
                        sleep(Duration::from_nanos(1));
                        polling_since = Instant::now();
                    }
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
