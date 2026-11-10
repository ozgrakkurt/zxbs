use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

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

        ThreadPool { workers, senders, next: AtomicUsize::new(0) }
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

pub struct TaskHandle<T> { rx: mpsc::Receiver<T> }

impl<T> TaskHandle<T> {
    pub fn join(self) -> T {
        self.rx.recv().expect("worker panicked before sending a result")
    }
}
