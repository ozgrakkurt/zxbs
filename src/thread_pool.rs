use crossbeam_channel::{Receiver, Sender, unbounded};
use std::thread;

type Job = Box<dyn FnOnce() + Send + 'static>;

pub struct ThreadPool {
    senders: Vec<Sender<Job>>,
}

impl ThreadPool {
    pub fn new(num_threads: usize, num_layers: usize) -> Self {
        assert!(num_threads > 0);
        let mut senders = Vec::with_capacity(num_layers);

        for _ in 0..num_layers {
            let (tx, rx) = unbounded::<Job>();
            senders.push(tx);

            for _ in 0..num_threads {
                let rx = rx.clone();
                thread::spawn(move || {
                    while let Ok(job) = rx.recv() {
                        job();
                    }
                });
            }
        }

        ThreadPool { senders }
    }

    fn spawn_with_tx<F, T>(&self, layer: usize, tx: Sender<T>, f: F)
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let job: Job = Box::new(move || {
            let _ = tx.send(f());
        });

        self.senders[layer].send(job).expect("worker thread exited");
    }

    pub fn spawn<F, T>(&self, layer: usize, f: F) -> TaskHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = unbounded::<T>();

        self.spawn_with_tx(layer, tx, f);

        TaskHandle { rx }
    }
}

pub struct TaskHandle<T> {
    rx: Receiver<T>,
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

    pub fn spawn_with_tx<F, T>(&self, tx: Sender<T>, f: F)
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.pool.spawn_with_tx(self.layer, tx, f);
    }

    pub fn spawn<F, T>(&self, f: F) -> TaskHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.pool.spawn(self.layer, f)
    }
}

pub struct JoinSet<T> {
    rx: Receiver<T>,
    tx: Sender<T>,
    len: usize,
}

impl<T> JoinSet<T> {
    pub fn new() -> Self {
        let (tx, rx) = unbounded::<T>();
        Self { tx, rx, len: 0 }
    }

    pub fn spawn<F>(&mut self, thread_pool_handle: ThreadPoolHandle, f: F)
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        thread_pool_handle.spawn_with_tx(self.tx.clone(), f);
        self.len += 1;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn join_next(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }

        let v = self.rx.recv().unwrap();
        self.len -= 1;

        Some(v)
    }

    pub fn join_all(&mut self) -> Vec<T> {
        let mut out = Vec::with_capacity(self.len());

        while let Some(v) = self.join_next() {
            out.push(v);
        }

        out
    }
}
