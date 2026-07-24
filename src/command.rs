pub type StdOut = String;

pub struct PrecompileToPcm {
    pub cxx: &'static str,
    pub cxxflags: &'static [&'static str],
    pub prebuilt_module_path_flag: &'static str,
    pub input: &'static str,
    pub output: &'static str,
}

impl PrecompileToPcm {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("<cxx> <cxxflags> ");

        out.push_str(self.input);

        out.push_str(b" --precompile <prebuilt_module_path_flag> -o ");

        out.push_str(self.output);

        out
    }

    pub async fn run(&self) -> StdOut {
        let output = match smol::process::Command::new(self.cxx)
            .args(self.cxxflags)
            .arg(self.input)
            .arg("--precompile")
            .arg(self.prebuilt_module_path_flag)
            .arg("-o")
            .arg(self.output)
            .output()
            .await
        {
            Ok(out) => out,
            Err(e) => {
                panic!(
                    "Failed to run command. Command:\n\t{}\nError = {:?}\n",
                    self.print(path_interner),
                    e,
                );
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(output.stderr);
            panic!(
                "Precompile to .pcm Failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(path_interner),
                output.status,
                stderr,
            );
        }

        String::from_utf8_lossy(output.stdout)
    }
}

pub struct CompileToObject {
    pub cxx: &'static str,
    pub cxxflags: &'static [&'static str],
    pub prebuilt_module_path_flag: &'static str,
    pub input: &'static str,
    pub output: &'static str,
}

impl CompileToObject {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("<cxx> <cxxflags> ");

        out.push_str(self.input);

        out.push_str(" <prebuilt_module_path_flag> -c -o ");

        out.push_str(self.output);

        out
    }

    pub async fn run(&self) -> StdOut {
        let output = match smol::process::Command::new(self.cxx)
            .args(self.cxxflags)
            .arg(self.input)
            .arg(self.prebuilt_module_path_flag)
            .arg("-c")
            .arg("-o")
            .arg(self.output)
            .output()
            .await
        {
            Ok(out) => out,
            Err(e) => {
                panic!(
                    "Failed to run command. Command:\n\t{}\nError = {:?}\n",
                    self.print(),
                    e,
                );
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(output.stderr);
            panic!(
                "Compile to .o Failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(),
                output.status,
                stderr,
            );
        }

        String::from_utf8_lossy(output.stdout)
    }
}

pub struct ArchiveLib {
    pub ar: &'static str,
    pub output: &'static str,
    pub inputs: Vec<&'static str>,
}

impl ArchiveLib {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("<ar> crs ");

        out.push_str(self.output);

        for input in self.inputs.iter() {
            out.push(' ');
            out.push_str(input);
        }

        out
    }

    pub async fn run(&self, path_interner: &PathInterner) -> StdOut {
        let output = match smol::process::Command::new(self.ar)
            .arg("crs")
            .arg(self.output)
            .args(self.inputs.as_slice())
            .output()
            .await
        {
            Ok(out) => out,
            Err(e) => {
                panic!(
                    "Failed to run command. Command:\n\t{}\nError = {:?}\n",
                    self.print(),
                    e,
                );
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(output.stderr);
            panic!(
                "Archiving a library failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(),
                output.status,
                stderr,
            );
        }

        String::from_utf8_lossy(output.stdout)
    }
}

pub struct LinkBinary {
    pub cxx: &'static str,
    pub ldflags: &'static [&'static str],
    pub output: &'static str,
    pub object: &'static str,
    pub deps: Vec<&'static str>,
}

impl LinkBinary {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("<cxx> <ldflags> -o ");

        out.push_str(self.output);

        out.push(' ');
        out.push_str(self.object);

        for dep in self.deps.iter() {
            out.push(' ');
            out.push_str(dep);
        }
    }

    pub async fn run(&self) -> StdOut {
        let output = match smol::process::Command::new(self.cxx)
            .args(self.ldflags)
            .arg("-o")
            .arg(self.output)
            .arg(self.object)
            .args(self.deps.as_slice())
            .output()
            .await
        {
            Ok(out) => out,
            Err(e) => {
                panic!(
                    "Failed to run command. Command:\n\t{}\nError = {:?}\n",
                    self.print(),
                    e,
                );
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(output.stderr);
            panic!(
                "Linking a binary failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(),
                output.status,
                stderr,
            );
        }

        String::from_utf8_lossy(output.stdout)
    }
}

pub struct RunBinary {
    pub path: &'static str,
}

impl RunBinary {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("RUN_BINARY ");
        out.push_str(self.path);

        out
    }

    pub async fn run(&self) -> StdOut {
        let output = match smol::process::Command::new(self.path).output().await {
            Ok(out) => out,
            Err(e) => {
                panic!(
                    "Failed to run command. Command:\n\t{}\nError = {:?}\n",
                    self.print(),
                    e,
                );
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(output.stderr);
            panic!(
                "Running a binary failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(),
                output.status,
                stderr,
            );
        }

        String::from_utf8_lossy(output.stdout)
    }
}

pub enum Command {
    PrecompileToPcm(Box<PrecompileToPcm>),
    CompileToObject(Box<CompileToObject>),
    ArchiveLib(Box<ArchiveLib>),
    LinkBinary(Box<LinkBinary>),
    RunBinary(RunBinary),
}

impl Command {
    pub fn print(&self) -> String {
        match self {
            Command::PrecompileToPcm(x) => x.print(),
            Command::CompileToObject(x) => x.print(),
            Command::ArchiveLib(x) => x.print(),
            Command::LinkBinary(x) => x.print(),
            Command::RunBinary(x) => x.print(),
        }
    }

    pub async fn run(&self) -> StdOut {
        match self {
            Command::PrecompileToPcm(x) => x.run().await,
            Command::CompileToObject(x) => x.run().await,
            Command::ArchiveLib(x) => x.run().await,
            Command::LinkBinary(x) => x.run().await,
            Command::RunBinary(x) => x.run().await,
        }
    }
}
