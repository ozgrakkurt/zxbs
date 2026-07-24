pub struct PrecompileToPcm {
    pub cxx: &'static str,
    pub cxxflags: &'static [&'static str],
    pub input: &'static str,
    pub output: &'static str,
}

impl PrecompileToPcm {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("<cxx> <cxxflags> ");

        out.push_str(self.input);

        out.push_str(" --precompile -o ");

        out.push_str(self.output);

        out
    }

    pub async fn run(&self) {
        let output = match smol::process::Command::new(self.cxx)
            .args(self.cxxflags)
            .arg(self.input)
            .arg("--precompile")
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
            let stderr = String::from_utf8(output.stderr).unwrap();
            panic!(
                "Precompile to .pcm Failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(),
                output.status,
                stderr,
            );
        }

        let stdout = String::from_utf8(output.stdout).unwrap();
        if !stdout.trim().is_empty() {
            eprintln!("STDOUT of {}:\n{}", self.print(), stdout.as_str());
        }
    }
}

pub struct CompileToObject {
    pub cxx: &'static str,
    pub cxxflags: &'static [&'static str],
    pub input: &'static str,
    pub output: &'static str,
}

impl CompileToObject {
    pub fn print(&self) -> String {
        let mut out = String::with_capacity(1024);

        out.push_str("<cxx> <cxxflags> ");

        out.push_str(self.input);

        out.push_str(" -c -o ");

        out.push_str(self.output);

        out
    }

    pub async fn run(&self) {
        let output = match smol::process::Command::new(self.cxx)
            .args(self.cxxflags)
            .arg(self.input)
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
            let stderr = String::from_utf8(output.stderr).unwrap();
            panic!(
                "Compile to .o Failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(),
                output.status,
                stderr,
            );
        }

        let stdout = String::from_utf8(output.stdout).unwrap();
        if !stdout.trim().is_empty() {
            eprintln!("STDOUT of {}:\n{}", self.print(), stdout.as_str());
        }
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

    pub async fn run(&self) {
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
            let stderr = String::from_utf8(output.stderr).unwrap();
            panic!(
                "Archiving a library failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(),
                output.status,
                stderr,
            );
        }

        let stdout = String::from_utf8(output.stdout).unwrap();
        if !stdout.trim().is_empty() {
            eprintln!("STDOUT of {}:\n{}", self.print(), stdout.as_str());
        }
    }
}

pub struct LinkBinary {
    pub cxx: &'static str,
    pub ldflags: &'static [&'static str],
    pub output: &'static str,
    pub object: &'static str,
    pub deps: &'static [&'static str],
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

        out
    }

    pub async fn run(&self) {
        let output = match smol::process::Command::new(self.cxx)
            .args(self.ldflags)
            .arg("-o")
            .arg(self.output)
            .arg(self.object)
            .args(self.deps)
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
            let stderr = String::from_utf8(output.stderr).unwrap();
            panic!(
                "Linking a binary failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(),
                output.status,
                stderr,
            );
        }

        let stdout = String::from_utf8(output.stdout).unwrap();
        if !stdout.trim().is_empty() {
            eprintln!("STDOUT of {}:\n{}", self.print(), stdout.as_str());
        }
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

    pub async fn run(&self) {
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
            let stderr = String::from_utf8(output.stderr).unwrap();
            panic!(
                "Running a binary failed. Command:\n\t{}\nStatus: {}\nStdErr:\n{}\n",
                self.print(),
                output.status,
                stderr,
            );
        }

        let stdout = String::from_utf8(output.stdout).unwrap();
        if !stdout.trim().is_empty() {
            eprintln!("STDOUT of {}:\n{}", self.print(), stdout.as_str());
        }
    }
}

pub enum Command {
    PrecompileToPcm(PrecompileToPcm),
    CompileToObject(CompileToObject),
    ArchiveLib(ArchiveLib),
    LinkBinary(LinkBinary),
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

    pub async fn run(&self) {
        match self {
            Command::PrecompileToPcm(x) => x.run().await,
            Command::CompileToObject(x) => x.run().await,
            Command::ArchiveLib(x) => x.run().await,
            Command::LinkBinary(x) => x.run().await,
            Command::RunBinary(x) => x.run().await,
        }
    }
}
