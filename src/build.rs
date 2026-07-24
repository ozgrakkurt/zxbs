pub struct Task {
    pub inputs: Vec<Str>,
    pub outputs: Vec<Str>,
    pub command: Command,
}

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

pub type Executor = smol::Executor<'static>;

#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Build {
    pub executor: &'static Executor,
    pub is_built: Vec<bool>,
    pub dependencies: Vec<Vec<Str>>,
    pub dependants: Vec<Vec<TaskIndex>>,
    pub tasks: Vec<Task>,
    pub running: Vec<smol::Task<TaskIndex>>,
    pub backlog: VecDeque<TaskIndex>,
    pub concurrency: usize,
    pub n_tasks_finished: usize,
    pub done: bool,
}

impl Build {
    fn new(executor: &'static Executor, tasks: Vec<Task>) -> Build {
        let mut dependencies = Vec::with_capacity(tasks.len());

        let mut dependants = Vec::with_capacity(tasks.len());

        let mut backlog = VecDeque::new();

        for (idx, task) in tasks.iter().enumerate() {
            let task_idx = TaskIndex::from_usize(idx);
            dependencies.push(task.input_indices.to_vec());
            for input_idx in task.input_indices.iter() {
                dependants.resize(input_idx.to_usize() + 1, Vec::new());
                dependants[input_idx.to_usize()].push(task_idx);
            }

            for output_idx in task.output_indices.iter() {
                dependants.resize(output_idx.to_usize() + 1, Vec::new());
            }

            if task.input_indices.is_empty() {
                backlog.push_back(task_idx);
            }
        }

        Build {
            executor,
            is_built: vec![false; tasks.len()],
            dependencies,
            dependants,
            tasks,
            running: Vec::new(),
            backlog,
            concurrency: num_cpus::get(),
            n_tasks_finished: 0,
            done: false,
        }
    }

    fn process_task_finished(&mut self, task_idx: TaskIndex) {
        let task = &self.tasks[task_idx.to_usize()];
        self.n_tasks_finished += 1;
        for out_idx in task.output_indices.iter() {
            assert!(!self.is_built[out_idx.to_usize()]);
            self.is_built[out_idx.to_usize()] = true;

            for &dependant_idx in self.dependants[out_idx.to_usize()].iter() {
                let deps: &mut Vec<PathIndex> = &mut self.dependencies[dependant_idx.to_usize()];
                let pos = deps.iter().position(|idx| idx.0 == out_idx.0).unwrap();
                deps.swap_remove(pos);

                if deps.is_empty() {
                    self.backlog.push_back(dependant_idx);
                }
            }
        }
    }
}

impl Future for Build {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let fut = self.get_mut();

        assert!(!fut.done);

        let mut idx = 0;
        while idx < fut.running.len() {
            let job = &mut fut.running[idx];
            match job.poll(cx) {
                Poll::Ready(task_idx) => {
                    fut.process_task_finished(task_idx);
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

            let command = fut.tasks[task_idx.to_usize()].command.take().unwrap();
            let mut handle = fut.executor.spawn(command);

            match handle.poll(cx) {
                Poll::Ready(task_idx) => {
                    fut.process_task_finished(task_idx);
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
