use crossbeam_channel::{Receiver, Sender, unbounded};
use std::thread;

type Job = Box<dyn FnOnce() + Send + 'static>;

pub struct ThreadPool {
    sender: Sender<Job>,
}

impl ThreadPool {
    pub fn new(num_threads: usize) -> Self {
        assert!(num_threads > 0);
        let (tx, rx) = unbounded::<Job>();

        for _ in 0..num_threads {
            let rx = rx.clone();
            thread::spawn(move || {
                while let Ok(job) = rx.recv() {
                    job();
                }
            });
        }

        ThreadPool { sender: tx }
    }

    fn spawn<F, T>(&self, tx: Sender<T>, f: F)
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let job: Job = Box::new(move || {
            let _ = tx.send(f());
        });

        self.sender.send(job).unwrap();
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

    pub fn spawn<F>(&mut self, thread_pool: &ThreadPool, f: F)
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        thread_pool.spawn(self.tx.clone(), f);
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
