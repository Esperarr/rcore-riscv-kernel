# Project Overview

## Motivation

The project turns an operating-systems course into a sequence of boundary
crossings that can be inspected directly: user code enters the kernel through
traps, virtual addresses are translated through page tables, processes move
through schedulers, file operations mutate a block device, and concurrent
threads coordinate through kernel-managed objects.

Each assessed lab adds one subsystem while preserving the course baseline. The
goal of this portfolio presentation is not to relabel shared framework code as
personal work. It is to make the individual extensions, the surrounding
architecture, and the development history easy to evaluate.

## Personal implementation scope

### Lab 1 - Syscall tracing

The kernel records syscall activity in a two-level map keyed by task ID and
syscall ID. `SYS_trace` supports three operations:

- read one byte from a user-provided address;
- write one byte to a user-provided address;
- query a syscall count for the current task.

The syscall path increments the active task's counter before dispatch. This
stage uses the direct user mapping provided by the chapter-3 kernel; the later
virtual-memory lab replaces this pattern with explicit page-table translation.

### Lab 2 - Virtual memory operations

`SYS_get_time` constructs a time value in the kernel and copies its bytes into
one or more translated user buffers. This handles a `TimeVal` that crosses a
page boundary.

The memory manager implements a small `mmap`/`munmap` interface:

- require page-aligned start addresses and valid `R/W/X` protection bits;
- reject mappings that overlap currently valid pages;
- create backed virtual ranges with user-accessible permissions;
- validate an unmapped request against every page in its range;
- remove complete `MapArea` objects or split them into left and right
  survivors when only a middle range is removed.

This is an educational implementation. It does not implement demand paging,
swap, file-backed mappings, or copy-on-write.

### Lab 3 - Process creation and stride scheduling

`SYS_spawn` loads a named ELF directly into a fresh address space, allocates a
process ID and kernel stack, initializes a trap context, and adds the new task
to the ready queue. This avoids cloning an address space before replacing it,
which is the main unnecessary cost in implementing spawn as `fork` followed by
`exec`.

Each task carries `stride` and `priority` fields. The ready queue selects the
task with the smallest stride and increments it by
`BIG_STRIDE / priority`. `SYS_set_priority` rejects values below 2 and updates
the current task's weight.

### Lab 4 - Filesystem semantics

The on-disk inode gains an `nlink` reference count. The VFS then supports:

- `linkat`: find an existing inode, reject duplicate names, increment `nlink`,
  and append a directory entry;
- `unlinkat`: remove the requested entry, decrement `nlink`, and reclaim the
  inode and its data blocks at zero links;
- `fstat`: return device, inode number, file mode, and link count through a
  user-space `Stat` structure.

The implementation also fixes file-descriptor initialization in spawned
processes so stdin/stdout are available before the new program performs file
operations.

The filesystem is the course's single-directory `easy-fs`; it is not a full
POSIX filesystem.

### Lab 5 - Threads and deadlock detection

The kernel exposes mutex and semaphore creation, lock/down, unlock/up, and
condition-variable operations. Thread resources are attached to a process and
released as a group when the main thread exits.

For deadlock detection, each process tracks:

- available instances of each mutex or semaphore;
- allocation per thread and resource;
- additional need per thread and resource.

Before a blocking acquisition, the kernel simulates the proposed request and
repeatedly looks for a thread whose remaining needs can be satisfied. If every
thread can reach completion, the request proceeds; otherwise it is rolled back
and the syscall returns `-0xDEAD`.

The implementation is a request-time safety check for the teaching kernel, not
a general-purpose kernel transaction manager.

## Design choices

### Preserve history instead of flattening it

The original repository used one branch per chapter and merged later labs into
their immediate predecessors where applicable. Keeping those refs makes the
incremental work visible and avoids rewriting authorship or commit dates.

### Translate user buffers explicitly

Kernel code cannot safely dereference arbitrary user pointers. The implementation
uses the current page-table token to translate user ranges and copy data in
page-sized slices. This is especially important for structures or strings that
straddle page boundaries.

### Validate resource state before blocking

Deadlock checks are most useful before a thread enters an uninterruptible wait.
The synchronization syscalls update a trial resource state, run the safety
check, and only commit the acquisition if the resulting state is safe.

## Engineering skills demonstrated

- navigating and extending a multi-crate Rust kernel without relying on the
  standard library;
- debugging across user and supervisor privilege levels;
- reasoning about page tables, permissions, address spaces, and user pointers;
- managing process and resource lifetimes across task switches;
- implementing scheduler weights, filesystem reference counts, and
  resource-allocation safety checks;
- preserving reproducible development history for review.

## Verification and limits

The source is structured for the course's QEMU environment and official user
test suite. Runtime results are recorded in the lab reports on the original
branches. The full QEMU test harness depends on external test repositories and
course-specific checker assets and is not vendored into this repository.

The chapters intentionally remain separate in this snapshot. `ch8` contains the
threads/synchronization lab on its course baseline; it does not automatically
contain every change from `ch3` through `ch6`.
