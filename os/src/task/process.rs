//! Implementation of  [`ProcessControlBlock`]

use super::id::RecycleAllocator;
use super::manager::insert_into_pid2process;
use super::TaskControlBlock;
use super::{add_task, SignalFlags};
use super::{pid_alloc, PidHandle};
use crate::fs::{File, Stdin, Stdout};
use crate::mm::{translated_refmut, MemorySet, KERNEL_SPACE};
use crate::sync::{Condvar, Mutex, Semaphore, UPSafeCell};
use crate::trap::{trap_handler, TrapContext};
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefMut;

/// Process Control Block
pub struct ProcessControlBlock {
    /// immutable
    pub pid: PidHandle,
    /// mutable
    inner: UPSafeCell<ProcessControlBlockInner>,
}

/// Inner of Process Control Block
pub struct ProcessControlBlockInner {
    /// is zombie?
    pub is_zombie: bool,
    /// memory set(address space)
    pub memory_set: MemorySet,
    /// parent process
    pub parent: Option<Weak<ProcessControlBlock>>,
    /// children process
    pub children: Vec<Arc<ProcessControlBlock>>,
    /// exit code
    pub exit_code: i32,
    /// file descriptor table
    pub fd_table: Vec<Option<Arc<dyn File + Send + Sync>>>,
    /// signal flags
    pub signals: SignalFlags,
    /// tasks(also known as threads)
    pub tasks: Vec<Option<Arc<TaskControlBlock>>>,
    /// task resource allocator
    pub task_res_allocator: RecycleAllocator,
    /// mutex list
    pub mutex_list: Vec<Option<Arc<dyn Mutex>>>,
    /// semaphore list
    pub semaphore_list: Vec<Option<Arc<Semaphore>>>,
    /// condvar list
    pub condvar_list: Vec<Option<Arc<Condvar>>>,
    /// mutex info
    pub mutex_info: MutexResourceMatrix,
    /// semaphore info
    pub semaphore_info: SemaphoreResourceMatrix,
    /// 死锁检测开关
    pub deadlock_detection_enabled: bool,
}
/// 互斥锁相关资源状态
pub struct MutexResourceMatrix {
    pub available: Vec<Option<u8>>,
    pub allocation: Vec<Option<Vec<Option<u8>>>>,
    pub need: Vec<Option<Vec<Option<u8>>>>,
}

/// 信号量相关资源状态
pub struct SemaphoreResourceMatrix {
    pub available: Vec<Option<u8>>,
    pub allocation: Vec<Option<Vec<Option<u8>>>>,
    pub need: Vec<Option<Vec<Option<u8>>>>,
}

impl ProcessControlBlockInner {
    #[allow(unused)]
    /// get the address of app's page table
    pub fn get_user_token(&self) -> usize {
        self.memory_set.token()
    }
    /// allocate a new file descriptor
    pub fn alloc_fd(&mut self) -> usize {
        if let Some(fd) = (0..self.fd_table.len()).find(|fd| self.fd_table[*fd].is_none()) {
            fd
        } else {
            self.fd_table.push(None);
            self.fd_table.len() - 1
        }
    }
    /// allocate a new task id
    pub fn alloc_tid(&mut self) -> usize {
        self.task_res_allocator.alloc()
    }
    /// deallocate a task id
    pub fn dealloc_tid(&mut self, tid: usize) {
        self.task_res_allocator.dealloc(tid)
    }
    /// the count of tasks(threads) in this process
    pub fn thread_count(&self) -> usize {
        self.tasks.len()
    }
    /// get a task with tid in this process
    pub fn get_task(&self, tid: usize) -> Arc<TaskControlBlock> {
        self.tasks[tid].as_ref().unwrap().clone()
    }
    /// 检查互斥锁请求是否安全
    pub fn check_mutex_safety(&self, _tid: usize, mutex_id: usize) -> bool {
        if !self.deadlock_detection_enabled {
            return true;
        }
        // 检查是否有足够的资源
        if let Some(avail) = self.mutex_info.available.get(mutex_id) {
            if avail.unwrap_or(0) > 0 {
                return true;
            }
        }
        // 检查是否有线程持有该资源
        let mut has_holder = false;
        for thread_id in 0..self.mutex_info.allocation.len() {
            if let Some(alloc) = &self.mutex_info.allocation[thread_id] {
                if let Some(val) = alloc.get(mutex_id) {
                    if val.unwrap_or(0) > 0 {
                        has_holder = true;
                        break;
                    }
                }
            }
        }
        if !has_holder {
            return true;
        }
        // 算法主体
        let mut work = self.mutex_info.available.clone();
        let mut finish = vec![false; self.mutex_info.allocation.len()];
        // 首先标记所有不需要资源的线程为完成
        for thread_id in 0..self.mutex_info.allocation.len() {
            if let (Some(alloc), Some(need)) = (&self.mutex_info.allocation[thread_id], &self.mutex_info.need[thread_id]) {
                let mut can_finish = true;
                for resource_id in 0..need.len() {
                    if need[resource_id].unwrap_or(0) > 0 {
                        can_finish = false;
                        break;
                    }
                }
                if can_finish {
                    finish[thread_id] = true;
                    // 释放该线程持有的资源
                    for resource_id in 0..alloc.len() {
                        if let Some(val) = work.get_mut(resource_id) {
                            *val = Some(val.unwrap_or(0) + alloc[resource_id].unwrap_or(0));
                        }
                    }
                }
            }
        }
        // 循环查找能满足条件的线程
        // 假设最多循环 N 次，避免死循环
        let max_rounds = self.mutex_info.allocation.len();
        for _round in 0..max_rounds {
            let mut found = false;
            for thread_id in 0..self.mutex_info.allocation.len() {
                if !finish[thread_id] {
                    if let (Some(alloc), Some(need)) = (&self.mutex_info.allocation[thread_id], &self.mutex_info.need[thread_id]) {
                        let mut can_allocate = true;
                        for resource_id in 0..need.len() {
                            if need[resource_id].unwrap_or(0) > work[resource_id].unwrap_or(0) {
                                can_allocate = false;
                                break;
                            }
                        }
                        if can_allocate {
                            finish[thread_id] = true;
                            found = true;
                            for resource_id in 0..alloc.len() {
                                if let Some(val) = work.get_mut(resource_id) {
                                    *val = Some(val.unwrap_or(0) + alloc[resource_id].unwrap_or(0));
                                }
                            }
                        }
                    }
                }
            }
            if !found {
                break;
            }
        }
        // 检查所有线程是否都已完成，如果是，则安全
        finish.iter().all(|&x| x)
    }
    /// 检查信号量请求是否安全
    pub fn check_semaphore_safety(&self, _tid: usize, _sem_id: usize) -> bool {
        // 如果死锁检测未启用，则默认安全
        if !self.deadlock_detection_enabled {
            return true;
        }
        let mut work = self.semaphore_info.available.clone();
        // 标记所有线程未完成
        let mut finish = vec![false; self.semaphore_info.allocation.len()];
        // 标记已经完成的线程
        // 如果线程当前不需要任何资源，则它可视为已完成，释放资源回 Work
        for i in 0..self.semaphore_info.allocation.len() {
            if let (Some(alloc), Some(need)) = (&self.semaphore_info.allocation[i], &self.semaphore_info.need[i]) {
                // 判断该线程是否无资源需求
                let no_need = need.iter().all(|&n| n.unwrap_or(0) == 0);
                if no_need {
                    finish[i] = true;
                    // 释放该线程已分配的资源
                    for j in 0..alloc.len() {
                        if let Some(w) = work.get_mut(j) {
                            *w = Some(w.unwrap_or(0) + alloc[j].unwrap_or(0));
                        }
                    }
                }
            }
        }
        // 循环查找能满足条件的线程
        // 假设最多循环 N 次，避免死循环
        for _round in 0..self.semaphore_info.allocation.len() {
            let mut progress = false;
            for i in 0..self.semaphore_info.allocation.len() {
                if !finish[i] {
                    if let (Some(_alloc), Some(need)) = (&self.semaphore_info.allocation[i], &self.semaphore_info.need[i]) {
                        let can_allocate = need.iter().enumerate().all(|(j, &req)| {
                            req.unwrap_or(0) <= work.get(j).unwrap_or(&Some(0)).unwrap_or(0)
                        });
                        if can_allocate {
                            finish[i] = true;
                            progress = true;
                            if let Some(alloc) = &self.semaphore_info.allocation[i] {
                                for j in 0..alloc.len() {
                                    if let Some(w) = work.get_mut(j) {
                                        *w = Some(w.unwrap_or(0) + alloc[j].unwrap_or(0));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if !progress {
                break;
            }
        }
        // 检查所有线程是否都已完成，如果是，则安全
        finish.iter().all(|&done| done)
    }
}
impl ProcessControlBlock {
    /// inner_exclusive_access
    pub fn inner_exclusive_access(&self) -> RefMut<'_, ProcessControlBlockInner> {
        self.inner.exclusive_access()
    }
    /// new process from elf file
    pub fn new(elf_data: &[u8]) -> Arc<Self> {
        trace!("kernel: ProcessControlBlock::new");
        // memory_set with elf program headers/trampoline/trap context/user stack
        let (memory_set, ustack_base, entry_point) = MemorySet::from_elf(elf_data);
        // allocate a pid
        let pid_handle = pid_alloc();
        let process = Arc::new(Self {
            pid: pid_handle,
            inner: unsafe {
                UPSafeCell::new(ProcessControlBlockInner {
                    is_zombie: false,
                    memory_set,
                    parent: None,
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: vec![
                        // 0 -> stdin
                        Some(Arc::new(Stdin)),
                        // 1 -> stdout
                        Some(Arc::new(Stdout)),
                        // 2 -> stderr
                        Some(Arc::new(Stdout)),
                    ],
                    signals: SignalFlags::empty(),
                    tasks: Vec::new(),
                    task_res_allocator: RecycleAllocator::new(),
                    mutex_list: Vec::new(),
                    semaphore_list: Vec::new(),
                    condvar_list: Vec::new(),
                    mutex_info: MutexResourceMatrix {
                        available: Vec::new(),
                        allocation: Vec::new(),
                        need: Vec::new(),
                    },
                    semaphore_info: SemaphoreResourceMatrix {
                        available: Vec::new(),
                        allocation: Vec::new(),
                        need: Vec::new(),
                    },
                    deadlock_detection_enabled: false,
                })
            },
        });
        // create a main thread, we should allocate ustack and trap_cx here
        let task = Arc::new(TaskControlBlock::new(
            Arc::clone(&process),
            ustack_base,
            true,
        ));
        // prepare trap_cx of main thread
        let task_inner = task.inner_exclusive_access();
        let trap_cx = task_inner.get_trap_cx();
        let ustack_top = task_inner.res.as_ref().unwrap().ustack_top();
        let kstack_top = task.kstack.get_top();
        drop(task_inner);
        *trap_cx = TrapContext::app_init_context(
            entry_point,
            ustack_top,
            KERNEL_SPACE.exclusive_access().token(),
            kstack_top,
            trap_handler as usize,
        );
        // add main thread to the process
        let mut process_inner = process.inner_exclusive_access();
        process_inner.tasks.push(Some(Arc::clone(&task)));
        process_inner.mutex_info.allocation.push(Some(Vec::new()));
        process_inner.mutex_info.need.push(Some(Vec::new()));
        process_inner.mutex_info.available.push(Some(0));
        process_inner.semaphore_info.allocation.push(Some(Vec::new()));
        process_inner.semaphore_info.need.push(Some(Vec::new()));
        process_inner.semaphore_info.available.push(Some(0));
        drop(process_inner);
        insert_into_pid2process(process.getpid(), Arc::clone(&process));
        // add main thread to scheduler
        add_task(task);
        process
    }

    /// Only support processes with a single thread.
    pub fn exec(self: &Arc<Self>, elf_data: &[u8], args: Vec<String>) {
        trace!("kernel: exec");
        assert_eq!(self.inner_exclusive_access().thread_count(), 1);
        // memory_set with elf program headers/trampoline/trap context/user stack
        trace!("kernel: exec .. MemorySet::from_elf");
        let (memory_set, ustack_base, entry_point) = MemorySet::from_elf(elf_data);
        let new_token = memory_set.token();
        // substitute memory_set
        trace!("kernel: exec .. substitute memory_set");
        self.inner_exclusive_access().memory_set = memory_set;
        // then we alloc user resource for main thread again
        // since memory_set has been changed
        trace!("kernel: exec .. alloc user resource for main thread again");
        let task = self.inner_exclusive_access().get_task(0);
        let mut task_inner = task.inner_exclusive_access();
        task_inner.res.as_mut().unwrap().ustack_base = ustack_base;
        task_inner.res.as_mut().unwrap().alloc_user_res();
        task_inner.trap_cx_ppn = task_inner.res.as_mut().unwrap().trap_cx_ppn();
        // push arguments on user stack
        trace!("kernel: exec .. push arguments on user stack");
        let mut user_sp = task_inner.res.as_mut().unwrap().ustack_top();
        user_sp -= (args.len() + 1) * core::mem::size_of::<usize>();
        let argv_base = user_sp;
        let mut argv: Vec<_> = (0..=args.len())
            .map(|arg| {
                translated_refmut(
                    new_token,
                    (argv_base + arg * core::mem::size_of::<usize>()) as *mut usize,
                )
            })
            .collect();
        *argv[args.len()] = 0;
        for i in 0..args.len() {
            user_sp -= args[i].len() + 1;
            *argv[i] = user_sp;
            let mut p = user_sp;
            for c in args[i].as_bytes() {
                *translated_refmut(new_token, p as *mut u8) = *c;
                p += 1;
            }
            *translated_refmut(new_token, p as *mut u8) = 0;
        }
        // make the user_sp aligned to 8B for k210 platform
        user_sp -= user_sp % core::mem::size_of::<usize>();
        // initialize trap_cx
        trace!("kernel: exec .. initialize trap_cx");
        let mut trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            KERNEL_SPACE.exclusive_access().token(),
            task.kstack.get_top(),
            trap_handler as usize,
        );
        trap_cx.x[10] = args.len();
        trap_cx.x[11] = argv_base;
        *task_inner.get_trap_cx() = trap_cx;
    }

    /// Only support processes with a single thread.
    pub fn fork(self: &Arc<Self>) -> Arc<Self> {
        trace!("kernel: fork");
        let mut parent = self.inner_exclusive_access();
        assert_eq!(parent.thread_count(), 1);
        // clone parent's memory_set completely including trampoline/ustacks/trap_cxs
        let memory_set = MemorySet::from_existed_user(&parent.memory_set);
        // alloc a pid
        let pid = pid_alloc();
        // copy fd table
        let mut new_fd_table: Vec<Option<Arc<dyn File + Send + Sync>>> = Vec::new();
        for fd in parent.fd_table.iter() {
            if let Some(file) = fd {
                new_fd_table.push(Some(file.clone()));
            } else {
                new_fd_table.push(None);
            }
        }
        // create child process pcb
        let child = Arc::new(Self {
            pid,
            inner: unsafe {
                UPSafeCell::new(ProcessControlBlockInner {
                    is_zombie: false,
                    memory_set,
                    parent: Some(Arc::downgrade(self)),
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: new_fd_table,
                    signals: SignalFlags::empty(),
                    tasks: Vec::new(),
                    task_res_allocator: RecycleAllocator::new(),
                    mutex_list: Vec::new(),
                    semaphore_list: Vec::new(),
                    condvar_list: Vec::new(),
                    mutex_info: MutexResourceMatrix {
                        available: Vec::new(),
                        allocation: Vec::new(),
                        need: Vec::new(),
                    },
                    semaphore_info: SemaphoreResourceMatrix {
                        available: Vec::new(),
                        allocation: Vec::new(),
                        need: Vec::new(),
                    },
                    deadlock_detection_enabled: parent.deadlock_detection_enabled,
                })
            },
        });
        // add child
        parent.children.push(Arc::clone(&child));
        // create main thread of child process
        let task = Arc::new(TaskControlBlock::new(
            Arc::clone(&child),
            parent
                .get_task(0)
                .inner_exclusive_access()
                .res
                .as_ref()
                .unwrap()
                .ustack_base(),
            // here we do not allocate trap_cx or ustack again
            // but mention that we allocate a new kstack here
            false,
        ));
        // attach task to child process
        let mut child_inner = child.inner_exclusive_access();
        child_inner.tasks.push(Some(Arc::clone(&task)));
        child_inner.mutex_info.allocation.push(Some(Vec::new()));
        child_inner.mutex_info.need.push(Some(Vec::new()));
        child_inner.mutex_info.available.push(Some(0));
        child_inner.semaphore_info.allocation.push(Some(Vec::new()));
        child_inner.semaphore_info.need.push(Some(Vec::new()));
        child_inner.semaphore_info.available.push(Some(0));
        drop(child_inner);
        // modify kstack_top in trap_cx of this thread
        let task_inner = task.inner_exclusive_access();
        let trap_cx = task_inner.get_trap_cx();
        trap_cx.kernel_sp = task.kstack.get_top();
        drop(task_inner);
        insert_into_pid2process(child.getpid(), Arc::clone(&child));
        // add this thread to scheduler
        add_task(task);
        child
    }
    /// get pid
    pub fn getpid(&self) -> usize {
        self.pid.0
    }
}