use std::collections::VecDeque;
use std::time::Instant;

use crate::command::Command;
use crate::interner::{Interner, Str};
use crate::thread_pool::{JoinSet, ThreadPool};
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

type TaskOutput = (Command, TaskIndex, u64, u64, usize);
type RunningSet = JoinSet<TaskOutput>;

#[must_use = "Constructing a build is useless unless it is finished by calling .build() on it"]
pub struct Build<'a> {
    tracer: Option<&'a mut tracer::Tracer>,
    interner: &'a mut Interner,
    is_built: Vec<bool>,
    dependencies: Vec<Vec<Str>>,
    dependants: Vec<Vec<TaskIndex>>,
    tasks: Vec<Task>,
    running: RunningSet,
    backlog: VecDeque<TaskIndex>,
    concurrency: usize,
    n_tasks_finished: usize,
    beginning: Instant,
    // used to give a unique temporary id to every currently running task.
    // This is useful for generating timing traces so we can imagine each task running in
    // its own thread.
    // Then we are able to visualize how much concurrency we are utilizing at a time and
    // which tasks are running concurrently with each other.
    free_slot_indices: Vec<usize>,
    thread_pool: &'a ThreadPool,
}

impl<'a> Build<'a> {
    pub fn new(
        tracer: Option<&'a mut tracer::Tracer>,
        interner: &'a mut Interner,
        tasks: Vec<Task>,
        beginning: Instant,
        thread_pool: &'a ThreadPool,
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
            running: JoinSet::new(),
            free_slot_indices,
            backlog,
            concurrency,
            n_tasks_finished: 0,
            beginning,
            thread_pool,
        }
    }

    fn process_task_finished(&mut self, (cmd, task_idx, start, duration, slot_index): TaskOutput) {
        let task = &self.tasks[task_idx.to_usize()];
        self.n_tasks_finished += 1;

        self.free_slot_indices.push(slot_index);

        if let Some(tracer) = self.tracer.as_mut() {
            tracer.record(tracer::Entry {
                start,
                duration,
                kind: cmd.kind(),
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

    pub fn build(mut self) {
        loop {
            if let Some(task_idx) = self.backlog.pop_front() {
                while self.running.len() >= self.concurrency {
                    let output = self.running.join_next().unwrap();
                    self.process_task_finished(output);
                }

                let command = self.tasks[task_idx.to_usize()].command.take().unwrap();
                let beginning = self.beginning;
                let slot_index = self.free_slot_indices.pop().unwrap();

                self.running.spawn(self.thread_pool, move || {
                    let task_start = u64::try_from(beginning.elapsed().as_micros()).unwrap();
                    let task_begin = Instant::now();
                    command.run();
                    let duration = u64::try_from(task_begin.elapsed().as_micros()).unwrap();
                    (command, task_idx, task_start, duration, slot_index)
                });
            } else if let Some(output) = self.running.join_next() {
                self.process_task_finished(output);
            } else {
                break;
            }
        }

        while let Some(output) = self.running.join_next() {
            self.process_task_finished(output);
        }

        assert!(self.n_tasks_finished == self.tasks.len());
    }
}
