//! Xernel build & run automation.
//!
//! Run via `cargo xtask <subcommand>` from the workspace root.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "xtask",
    about = "Xernel build & run automation",
    version,
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Build the kernel for the given architecture.
    Build {
        #[arg(long, value_enum, default_value_t = Arch::X86_64)]
        arch: Arch,
        #[arg(long)]
        release: bool,
    },
    /// Build a bootable ISO image with the Limine bootloader.
    Iso {
        #[arg(long, value_enum, default_value_t = Arch::X86_64)]
        arch: Arch,
        #[arg(long)]
        release: bool,
        /// Use this external ELF as the `init.elf` boot module instead of
        /// building `userland/init`. Lets a separate userland (e.g. XOS) boot
        /// without touching the kernel tree.
        #[arg(long, value_name = "PATH")]
        init: Option<PathBuf>,
    },
    /// Build the ISO and launch QEMU.
    Run {
        #[arg(long, value_enum, default_value_t = Arch::X86_64)]
        arch: Arch,
        #[arg(long)]
        release: bool,
        /// Wait for a GDB connection on :1234 before starting execution.
        #[arg(long)]
        gdb: bool,
        /// Enable KVM acceleration (host must support it).
        #[arg(long)]
        kvm: bool,
        /// Build with the `boot-test` feature, wire up isa-debug-exit, run
        /// headless, and turn the QEMU exit status into a pass/fail result.
        #[arg(long)]
        test: bool,
        /// Use this external ELF as the `init.elf` boot module instead of
        /// building `userland/init`.
        #[arg(long, value_name = "PATH")]
        init: Option<PathBuf>,
    },
    /// Fetch the Limine bootloader binaries into `boot/limine/`.
    FetchLimine,
    /// Remove all build artifacts.
    Clean,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Arch {
    X86_64,
    Aarch64,
    Riscv64,
}

impl Arch {
    fn triple(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64-xernel",
            Self::Aarch64 => "aarch64-xernel",
            Self::Riscv64 => "riscv64-xernel",
        }
    }

    fn target_json(self, workspace: &Path) -> PathBuf {
        workspace.join("targets").join(format!("{}.json", self.triple()))
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let workspace = workspace_root()?;
    match cli.cmd {
        Cmd::Build { arch, release } => build_kernel(&workspace, arch, release, false).map(|_| ()),
        Cmd::Iso { arch, release, init } => {
            build_iso(&workspace, arch, release, false, init.as_deref()).map(|_| ())
        }
        Cmd::Run { arch, release, gdb, kvm, test, init } => {
            run_qemu(&workspace, arch, release, gdb, kvm, test, init.as_deref())
        }
        Cmd::FetchLimine => fetch_limine(&workspace),
        Cmd::Clean => clean(&workspace),
    }
}

fn workspace_root() -> Result<PathBuf> {
    // `cargo xtask` runs the xtask binary with CWD = workspace root.
    let cwd = std::env::current_dir().context("read current dir")?;
    Ok(cwd)
}

fn build_kernel(workspace: &Path, arch: Arch, release: bool, test: bool) -> Result<PathBuf> {
    let target_json = arch.target_json(workspace);
    if !target_json.exists() {
        bail!(
            "target JSON for {:?} not found at {}",
            arch,
            target_json.display()
        );
    }
    let mut cmd = Command::new("cargo");
    cmd.current_dir(workspace)
        .arg("build")
        .arg("--package")
        .arg("kernel")
        .arg("--target")
        .arg(&target_json)
        .arg("-Z")
        .arg("json-target-spec")
        .arg("-Z")
        .arg("build-std=core,compiler_builtins,alloc")
        .arg("-Z")
        .arg("build-std-features=compiler-builtins-mem");
    if test {
        cmd.arg("--features").arg("boot-test");
    }
    if release {
        cmd.arg("--release");
    }
    let status = cmd.status().context("invoke cargo")?;
    if !status.success() {
        bail!("kernel build failed");
    }
    let profile = if release { "release" } else { "debug" };
    let elf = workspace
        .join("target")
        .join(arch.triple())
        .join(profile)
        .join("kernel");
    Ok(elf)
}

/// Build the `init` user program into a static ELF for the user target. The
/// linker script (placing it at a fixed base) is passed via RUSTFLAGS so the
/// crate needs no build.rs and the path stays unambiguous.
fn build_init(workspace: &Path, release: bool) -> Result<PathBuf> {
    let target_json = workspace.join("targets").join("x86_64-xernel-user.json");
    let linker = workspace.join("userland").join("init").join("linker.ld");
    let mut cmd = Command::new("cargo");
    cmd.current_dir(workspace)
        .env("RUSTFLAGS", format!("-C link-arg=-T{}", linker.display()))
        .arg("build")
        .arg("--package")
        .arg("init")
        .arg("--target")
        .arg(&target_json)
        .arg("-Z")
        .arg("json-target-spec")
        .arg("-Z")
        .arg("build-std=core,compiler_builtins")
        .arg("-Z")
        .arg("build-std-features=compiler-builtins-mem");
    if release {
        cmd.arg("--release");
    }
    let status = cmd.status().context("invoke cargo (init)")?;
    if !status.success() {
        bail!("init build failed");
    }
    let profile = if release { "release" } else { "debug" };
    Ok(workspace
        .join("target")
        .join("x86_64-xernel-user")
        .join(profile)
        .join("init"))
}

fn build_iso(
    workspace: &Path,
    arch: Arch,
    release: bool,
    test: bool,
    init_override: Option<&Path>,
) -> Result<PathBuf> {
    let elf = build_kernel(workspace, arch, release, test)?;
    // Either use the caller-supplied external init ELF, or build `userland/init`.
    let init_elf = match init_override {
        Some(path) => {
            if !path.exists() {
                bail!("--init path does not exist: {}", path.display());
            }
            println!("xtask: using external init module: {}", path.display());
            path.to_path_buf()
        }
        None => build_init(workspace, release)?,
    };
    let limine_dir = workspace.join("boot").join("limine");
    if !limine_dir.exists() {
        bail!(
            "Limine binaries missing — run `cargo xtask fetch-limine` first ({} not found)",
            limine_dir.display()
        );
    }
    let iso_root = workspace.join("target").join("iso_root");
    let _ = fs_err::remove_dir_all(&iso_root);
    fs_err::create_dir_all(iso_root.join("boot"))?;
    fs_err::copy(&elf, iso_root.join("boot").join("xernel.elf"))?;
    fs_err::copy(&init_elf, iso_root.join("boot").join("init.elf"))?;
    fs_err::copy(
        workspace.join("boot").join("limine.cfg"),
        iso_root.join("limine.cfg"),
    )?;
    for f in ["limine-bios.sys", "limine-bios-cd.bin", "limine-uefi-cd.bin"] {
        let src = limine_dir.join(f);
        if src.exists() {
            fs_err::copy(&src, iso_root.join(f))?;
        }
    }
    let iso_out = workspace.join("target").join("xernel.iso");
    let xorriso = which::which("xorriso").context("xorriso not found in PATH")?;
    let status = Command::new(xorriso)
        .args([
            "-as",
            "mkisofs",
            "-b",
            "limine-bios-cd.bin",
            "-no-emul-boot",
            "-boot-load-size",
            "4",
            "-boot-info-table",
            "--efi-boot",
            "limine-uefi-cd.bin",
            "-efi-boot-part",
            "--efi-boot-image",
            "--protective-msdos-label",
        ])
        .arg(&iso_root)
        .arg("-o")
        .arg(&iso_out)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("invoke xorriso")?;
    if !status.success() {
        bail!("xorriso failed");
    }
    Ok(iso_out)
}

/// Exit status QEMU reports when the kernel writes `ExitCode::Success` (0x10)
/// to the isa-debug-exit port: `(0x10 << 1) | 1`.
const QEMU_TEST_SUCCESS: i32 = 33;

fn run_qemu(
    workspace: &Path,
    arch: Arch,
    release: bool,
    gdb: bool,
    kvm: bool,
    test: bool,
    init_override: Option<&Path>,
) -> Result<()> {
    let iso = build_iso(workspace, arch, release, test, init_override)?;
    let qemu = match arch {
        Arch::X86_64 => "qemu-system-x86_64",
        Arch::Aarch64 => "qemu-system-aarch64",
        Arch::Riscv64 => "qemu-system-riscv64",
    };
    let qemu = which::which(qemu).with_context(|| format!("{qemu} not in PATH"))?;
    let mut cmd = Command::new(qemu);
    match arch {
        Arch::X86_64 => {
            cmd.args(["-M", "q35", "-m", "512M", "-cdrom"])
                .arg(&iso)
                .args(["-serial", "stdio", "-no-reboot"]);
            if test {
                cmd.args([
                    "-device",
                    "isa-debug-exit,iobase=0xf4,iosize=0x04",
                    "-display",
                    "none",
                ]);
            } else {
                cmd.args(["-no-shutdown", "-d", "int,guest_errors", "-D", "qemu.log"]);
            }
            if kvm {
                cmd.args(["-accel", "kvm", "-cpu", "host"]);
            }
        }
        Arch::Aarch64 | Arch::Riscv64 => {
            bail!("QEMU launch for {:?} not yet implemented", arch);
        }
    }
    if gdb {
        cmd.args(["-s", "-S"]);
    }
    let status = cmd.status().context("invoke qemu")?;
    if test {
        return match status.code() {
            Some(QEMU_TEST_SUCCESS) => {
                eprintln!("xtask: boot-test PASSED");
                Ok(())
            }
            other => bail!("boot-test FAILED (qemu exit code {other:?})"),
        };
    }
    if !status.success() {
        bail!("qemu exited with {status}");
    }
    Ok(())
}

fn fetch_limine(workspace: &Path) -> Result<()> {
    let target = workspace.join("boot").join("limine");
    if target.exists() {
        eprintln!("limine already present at {} — skipping", target.display());
        return Ok(());
    }
    let git = which::which("git").context("git not in PATH")?;
    let status = Command::new(git)
        .args([
            "clone",
            "https://github.com/limine-bootloader/limine.git",
            "--branch=v7.x-binary",
            "--depth=1",
        ])
        .arg(&target)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("invoke git clone")?;
    if !status.success() {
        bail!("git clone failed");
    }
    Ok(())
}

fn clean(workspace: &Path) -> Result<()> {
    let target = workspace.join("target");
    if target.exists() {
        fs_err::remove_dir_all(&target).context("remove target/")?;
    }
    Ok(())
}
