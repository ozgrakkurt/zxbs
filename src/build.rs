use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use crate::command::Command;
use crate::interner::{Interner, Str};
use crate::tracer;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TaskIndex(u32);

impl TaskIndex {
    pub fn from_usize(v: usize) -> TaskIndex {
        TaskIndex(u32::try_from(v).unwrap())
    }

    pub fn to_usize(&self) -> usize {
        usize::try_from(self.0).unwrap()
    }
}

pub struct Task {
    pub entry_name: Str,
    pub part_name: Str,
    pub inputs: Vec<Str>,
    pub outputs: Vec<Str>,
    pub command: Option<Command>,
    pub entry_index: usize,
}

type Handle = Pin<Box<tokio::task::JoinHandle<(Command, TaskIndex, u64, u64, usize)>>>;

#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Build<'a> {
    pub tracer: Option<&'a mut tracer::Tracer>,
    pub interner: &'a mut Interner,
    pub is_built: Vec<bool>,
    pub dependencies: Vec<Vec<Str>>,
    pub dependants: Vec<Vec<TaskIndex>>,
    pub tasks: Vec<Task>,
    pub running: Vec<Handle>,
    pub backlog: VecDeque<TaskIndex>,
    pub concurrency: usize,
    pub n_tasks_finished: usize,
    pub done: bool,
    pub beginning: Instant,
    // used to give a unique temporary id to every currently running task.
    // This is useful for generating timing traces so we can imagine each task running in
    // it's own thread.
    // Then we are able to visualize how much concurrency we are utilizing at a time and
    // which tasks are running concurrently with each other.
    pub free_slot_indices: Vec<usize>,
}

impl<'a> Build<'a> {
    pub fn new(
        tracer: Option<&'a mut tracer::Tracer>,
        interner: &'a mut Interner,
        tasks: Vec<Task>,
        beginning: Instant,
    ) -> Build<'a> {
        let mut dependencies = Vec::with_capacity(tasks.len());

        let mut dependants = vec![Vec::new(); interner.len()];

        let mut backlog = VecDeque::new();

        for (idx, task) in tasks.iter().enumerate() {
            let task_idx = TaskIndex::from_usize(idx);
            dependencies.push(task.inputs.to_vec());
            for input_idx in task.inputs.iter() {
                dependants[input_idx.to_usize()].push(task_idx);
            }

            if task.inputs.is_empty() {
                backlog.push_back(task_idx);
            }
        }

        if !tasks.is_empty() && backlog.is_empty() {
            panic!(
                "Couldn't find a starting task. All tasks have dependencies so there is no task to build at the start. Impossible to make progress."
            );
        }

        let is_built = vec![false; interner.len()];

        let concurrency = num_cpus::get();

        let free_slot_indices: Vec<usize> = (0..concurrency).rev().collect();

        Build {
            tracer,
            interner,
            is_built,
            dependencies,
            dependants,
            tasks,
            running: Vec::new(),
            free_slot_indices,
            backlog,
            concurrency,
            n_tasks_finished: 0,
            done: false,
            beginning,
        }
    }

    fn process_task_finished(
        &mut self,
        (cmd, task_idx, start, duration, slot_index): (Command, TaskIndex, u64, u64, usize),
    ) {
        let task = &self.tasks[task_idx.to_usize()];
        self.n_tasks_finished += 1;

        self.free_slot_indices.push(slot_index);

        if let Some(tracer) = self.tracer.as_mut() {
            let kind = self.interner.insert(cmd.kind_str());

            tracer.record(tracer::Entry {
                start,
                duration,
                kind,
                entry_name: task.entry_name,
                part_name: task.part_name,
                entry_index: task.entry_index,
                slot_index,
            });
        }

        for &out_idx in task.outputs.iter() {
            assert!(
                !self.is_built[out_idx.to_usize()],
                "The output {} was built for the second time by command: {}",
                self.interner.get(out_idx),
                cmd.print(),
            );
            self.is_built[out_idx.to_usize()] = true;

            for &dependant_idx in self.dependants[out_idx.to_usize()].iter() {
                let deps: &mut Vec<Str> = &mut self.dependencies[dependant_idx.to_usize()];
                let pos = deps.iter().position(|&idx| idx == out_idx).unwrap();
                deps.swap_remove(pos);

                if deps.is_empty() {
                    self.backlog.push_back(dependant_idx);
                }
            }
        }
    }
}

impl<'a> Future for Build<'a> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let fut = self.get_mut();

        assert!(!fut.done);

        let mut idx = 0;
        while idx < fut.running.len() {
            let job = &mut fut.running[idx];
            match job.as_mut().poll(cx) {
                Poll::Ready(res) => {
                    fut.process_task_finished(res.unwrap());
                    std::mem::drop(fut.running.swap_remove(idx));
                }
                Poll::Pending => {
                    idx += 1;
                }
            }
        }

        while fut.running.len() < fut.concurrency {
            let task_idx = match fut.backlog.pop_front() {
                Some(task_idx) => task_idx,
                None => break,
            };

            let mut command = fut.tasks[task_idx.to_usize()].command.take().unwrap();
            let task_start = u64::try_from(fut.beginning.elapsed().as_micros()).unwrap();
            let task_begin = Instant::now();
            let slot_index = fut.free_slot_indices.pop().unwrap();
            let mut handle = Box::pin(tokio::task::spawn(async move {
                command.run().await;
                let duration = u64::try_from(task_begin.elapsed().as_micros()).unwrap();
                (command, task_idx, task_start, duration, slot_index)
            }));

            match handle.as_mut().poll(cx) {
                Poll::Ready(res) => {
                    fut.process_task_finished(res.unwrap());
                }
                Poll::Pending => {
                    fut.running.push(handle);
                }
            }
        }

        if fut.running.is_empty() && fut.backlog.is_empty() {
            fut.done = true;
            assert_eq!(fut.n_tasks_finished, fut.tasks.len());

            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
