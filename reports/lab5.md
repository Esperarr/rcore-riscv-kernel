## Lab5 实验报告
周轻扬 计24 2022012166
## 实现功能
- 在 `ProcessControlBlockInner` 中添加布尔字段 `deadlock_detection_enable` 控制是否启用死锁检测。将互斥锁与信号量的资源信息封装为信息类 `mutex_info` 和 `semaphore_info`，分别包含 available、allocation、need 矩阵
- 修改系统调用：`sys_mutex_create` 初始化互斥锁资源信息；`sys_mutex_lock/unlock` 进行合法性检查和资源矩阵更新，lock对非法操作返回 -0xDEAD；信号量系统调用`sys_semaphore_down/up` `实现逻辑类似。
- 实现`enable_deadlock_detect`用于开关死锁检测开关
- 在`check_mutex_safety`和`check_semaphore_safety`中实现死锁检测算法，遍历线程，依据信息类模拟资源分配，判断是否存在所有线程都能完成的执行路径，从而判断当前操作是否合法
## 实验用时
- 阅读文档和代码框架：1.5h
- 实现：4h
- 调试：1.5-3h
- 报告：0.5h
## 问答题
### 1. 
#####  在我们的多线程实现中，当主线程 (即 0 号线程) 退出时，视为整个进程退出， 此时需要结束该进程管理的所有线程并回收其资源。 
##### - 需要回收的资源有哪些？
答：
- 未结束的所有子线程的TCB、锁、信号等资源信息、Trap上下文、用户栈、页表、tid等
- 进程的页表、内存空间、PCB、Trap上下文等

##### - 其他线程的 TaskControlBlock 可能在哪些位置被引用，分别是否需要回收，为什么？
答：可能在其他线程调度器、任务队列，或者锁/信号等资源的等待队列中被引用。不需要被回收，因为进程结束时会统一进行回收，不必额外手动回收

### 2. 

##### 对比以下两种 Mutex 中的实现，二者有什么区别？这些区别可能会导致什么问题？
```rust
 1 impl Mutex for Mutex1 {
 2    fn lock(&self) {
 3        loop {
 4            let mut mutex_inner = self.inner.exclusive_access();
 5            if mutex_inner.locked {
 6                mutex_inner.wait_queue.push_back(current_task().unwrap());
 7                drop(mutex_inner);
 8                block_current_and_run_next();
 9            } else {
10                mutex_inner.locked = true;
11                break;
12            }
13        }
14    }
15
16    fn unlock(&self) {
17        let mut mutex_inner = self.inner.exclusive_access();
18        assert!(mutex_inner.locked);
19        mutex_inner.locked = false;
20        if let Some(waking_task) = mutex_inner.wait_queue.pop_front() {
21            add_task(waking_task);
22        }
23    }
24}
25
26 impl Mutex for Mutex2 {
27    fn lock(&self) {
28        let mut mutex_inner = self.inner.exclusive_access();
29        if mutex_inner.locked {
30            mutex_inner.wait_queue.push_back(current_task().unwrap());
31            drop(mutex_inner);
32            block_current_and_run_next();
33        } else {
34            mutex_inner.locked = true;
35        }
36    }
37
38    fn unlock(&self) {
39        let mut mutex_inner = self.inner.exclusive_access();
40        assert!(mutex_inner.locked);
41        if let Some(waking_task) = mutex_inner.wait_queue.pop_front() {
42            add_task(waking_task);
43        } else {
44            mutex_inner.locked = false;
45        }
46    }
47}
```

答：
###### 区别：
1. lock() 方法
Mutex1: 使用 loop 循环重试，直到成功获取锁
Mutex2: 只尝试一次，失败后直接阻塞返回

2. unlock() 方法
Mutex1: 先设置locked = false、释放锁，再唤醒等待任务
Mutex2: 没有等待任务时才设置locked = false、释放锁，有等待任务时直接唤醒任务、不释放锁

###### 可能导致的问题:
1. Mutex1
被唤醒的任务可能重新竞争失败，反复阻塞-唤醒，导致唤醒-抢占-失败的循环；新任务可能抢占等待任务的锁
2. Mutex2
死锁风险: 如果被唤醒的任务异常退出，锁永远无法释放，从而导致死锁，后续所有任务都会永久等待

## 荣誉准则
1. 在完成本次实验的过程（含此前学习的过程）中，我曾分别与 以下各位 就（与本次实验相关的）以下方面做过交流，还在代码中对应的位置以注释形式记录了具体的交流对象及内容：
无
2. 此外，我也参考了 以下资料 ，还在代码中对应的位置以注释形式记录了具体的参考来源及内容：
实验文档<https://learningos.cn/rCore-Tutorial-Guide-2025S/chapter8>。
开始时由于代码框架较大，难以入手，在网络上查询资料后，借鉴了部分<https://github.com/1unx1/rCore/tree/ch8>和gpt提供的数据结构设计和维护思路，但仅借鉴部分思路，所有代码均为自己独立实现，并经过充分调试，在细节处理逻辑处进行了大量debug

1. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。 我清楚地知道，从以上方面获得的信息在一定程度上降低了实验难度，可能会影响起评分。

2. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。 我未曾也不会向他人（含此后各届同学）复制或公开我的实验代码，我有义务妥善保管好它们。 我提交至本实验的评测系统的代码，均无意于破坏或妨碍任何计算机系统的正常运转。 我清楚地知道，以上情况均为本课程纪律所禁止，若违反，对应的实验成绩将按“-100”分计。