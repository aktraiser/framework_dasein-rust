# Firecracker: controlled local beta

`FirecrackerSandbox` now starts a real microVM per `execute()`. It configures Firecracker with `--no-api --config-file`, waits for the guest's virtio-vsock listener, sends a length-prefixed JSON request, validates the correlated response, then kills/reaps the VM. There is no host-shell fallback.

Validated on an Apple M4 running macOS 15.5, with a Lima VZ Linux VM (`nestedVirtualization: true`), Linux 7.0/aarch64 hosting Firecracker 1.17.0 and a Linux 6.18.44 guest. A remote server is not necessary for this setup. Linux with accessible KVM is required; an arbitrary VPS or Docker on macOS does not guarantee KVM. Other CPUs, kernels and x86_64 still require their own qualification.

## Prepare an image

Use an operator-managed, trusted Python image matching the host architecture. The qualified image was `python:3.12-alpine`, digest `sha256:b64631e04e4920160c50fbe8d8df828f7f35f06f425cb44aa09bca53e708a35a`. For new images, repeat all tests. Export a stopped container:

```sh
docker create --name dasein-rootfs-export python:3.12-alpine
docker export -o python-rootfs.tar dasein-rootfs-export
docker rm dasein-rootfs-export
```

Copy the tar and this directory into Linux, install `e2fsprogs`, and build a new image (the script refuses to overwrite an existing output):

```sh
sudo bash server/firecracker-guest/build-rootfs.sh python-rootfs.tar /var/lib/firecracker/rootfs.ext4
```

Install the architecture-matching Firecracker release after verifying its published checksum, and a compatible uncompressed Linux kernel with ext4, virtio block and virtio-vsock built in. See the [official getting started guide](https://github.com/firecracker-microvm/firecracker/blob/main/docs/getting-started.md). Grant the service user read/write access to `/dev/kvm` through its `kvm` group; start a new login session after changing group membership. `/usr/bin/timeout` must implement coreutils-compatible process-group deadlines (the local tests used uutils 0.8.0).

## Run the SDK tests

```sh
export FIRECRACKER_BIN=/path/to/firecracker
export KERNEL_PATH=/path/to/vmlinux
export ROOTFS_PATH=/path/to/rootfs.ext4
cargo test -p dasein-agentic-sandbox --no-default-features --test firecracker_real -- --ignored --nocapture
```

The test covers guest UID/kernel, stdout/stderr, UTF-8, nonzero exits, fresh workspaces, read-only root, no network route, concurrent response correlation, output overflow, timeout, future cancellation, explicit stop and boot failure cleanup. It requires real KVM and never substitutes a host process. Ordinary non-Linux tests do not qualify this backend.

For application code, construct `FirecrackerConfig` with these paths and use `FirecrackerSandbox::with_config(config).execute(shell_command)`. Python is available in the prepared guest. Other runtimes require a new image and qualification. `is_ready()` checks prerequisites; execution additionally proves that the guest actually boots and answers.

## Isolation and lifecycle

- No virtual network interface or host directory is attached. Communication uses [virtio-vsock](https://github.com/firecracker-microvm/firecracker/blob/main/docs/vsock.md), guest port 5000.
- Root ext4 is read-only; `/workspace` (64 MiB) and `/tmp` (32 MiB) are new tmpfs mounts per execution. No filesystem persists between requests.
- The trusted broker is PID 1. Commands run as UID/GID 65534 with 64-process/64-file-descriptor limits, no setuid executables, and bounded output. Host limits are 1–8 vCPU, 32–2048 MiB RAM, 1–16 concurrent VMs per shared SDK instance, at most 120 seconds and 1 MiB combined output. Multiple independently constructed SDK instances need an application-wide capacity limit.
- Dropping an execution future or calling `stop()` signals destruction. The external coreutils deadline also kills the VM if the SDK process dies. A crash may leave a small private job directory; after confirming no VM is alive, the operator may remove stale directories. No automatic boot-time scavenger is claimed.
- This controlled beta uses Firecracker's default seccomp filtering, but **does not integrate the jailer, host cgroups, dedicated service-user provisioning or production multi-tenant hardening**. vCPU count is not a host CPU quota. Follow the official jailer/production-host guidance and qualify those controls before public untrusted workloads.

The HTTP adapter is in [firecracker-server](../firecracker-server/README.md). Gateway `Boxes` continue to use their qualified Docker backend; the legacy gateway warm-VM pool is not this implementation and remains disabled. This change does not claim warm snapshots, persistent VM filesystems or Rust toolchains in the Python image.
