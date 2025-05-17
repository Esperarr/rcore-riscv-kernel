## Lab3 实验报告
周轻扬 计24 2022012166
## 实现功能
1. sys_get_time：与ch4相同
2. sys_mmap、sys_munmap：由于改为使用processor进行进程管理，二者调用processor中实现的对应函数，操作当前进程memory_set，其余与ch4相同
3. sys_spawn：仿照sys_fork、sys_exec实现，当前进程加载elf并由当前进程调用spawn，创建新进程并加入就绪队列；spawn仿照new、fork实现由elf创建新进程并加入当前进程的children列表，仿照exec实现初始化trap_cx
4. sys_set_priority：TaskControlBlockInner中新增stride与priority属性（初始化时值为0、16）；sys_set_priority修改当前进程的priority；fetch中实现stride调度算法，选最小stride进程调度并设置该进程stride+=pass

## 问答题
### 1. 
stride 算法原理非常简单，但是有一个比较大的问题。例如两个 pass = 10 的进程，使用 8bit 无符号整形储存 stride， p1.stride = 255, p2.stride = 250，在 p2 执行一个时间片后，理论上下一次应该 p1 执行。
#####  实际情况是轮到 p1 执行吗？为什么？
答：不是。
当 p2 执行一个时间片后，p2.stride应该变成250+10=260，但由于使用 8bit 无符号整型，最大值为 255，所以会发生溢出，实际上 p2.stride 会变成 4。此时比较 p1.stride(255) 和 p2.stride(4)， 4 < 255，所以实际上下一次会继续执行 p2，而不是 p1。这与理论预期不符，因为理论上应该是 p1 执行。

### 2. 
我们之前要求进程优先级 >= 2 其实就是为了解决这个问题。可以证明， 在不考虑溢出的情况下 , 在进程优先级全部 >= 2 的情况下，如果严格按照算法执行，那么 STRIDE_MAX – STRIDE_MIN <= BigStride / 2。

##### 为什么？尝试简单说明（不要求严格证明）。
答：
当所有进程的优先级 >= 2 时，可以证明在不考虑溢出的情况下，如果严格按照算法执行，那么 STRIDE_MAX – STRIDE_MIN <= BigStride / 2。

简单说明：
数学归纳法证明：
###### 1. 基本假设
- 系统中有 n 个进程，优先级分别为 $p_1, p_2, ..., p_n$，且都 ≥ 2
- 每个进程的 pass 值为 $\frac{BigStride}{p_i}$
- 最大的 pass 值为 $\frac{BigStride}{2}$（当优先级为 2 时）
###### 2. 初始状态
假设初始（k=0）时所有进程的 stride 值都为 0（/或者任意相同的值）。此时显然满足：任意两个进程的 stride 差值为 0，小于最大 pass 值。
###### 3. 归纳步骤
假设在第 k+1 次调度前（经k次调度后），任意两个进程的 stride 差值不超过最大 pass 值（即 $\frac{BigStride}{2}$）。
在第 k+1 次调度中：
  1. 系统选择 stride 值最小的进程 A 执行
  2. 执行后，A 的 stride 值增加 pass_A
   
现在需要证明：在第 k+1 次调度前，任意两个进程的 stride 差值仍然不超过最大 pass 值。
对于任意两个进程 i 和 j，考虑以下情况：

  **情况1：如果 i 和 j 都不是进程 A**
  - 它们的 stride 值在第 k 次调度后没有变化
  - 根据归纳假设，它们的 stride 差值不超过最大 pass 值
  - 因此在第 k+1 次调度前，它们的 stride 差值仍然不超过最大 pass 值

  **情况2：如果 i 是进程 A，j 不是**
  - 第 k 次调度后，i 的 stride 值增加了 pass_i
  - 在第 k 次调度前，i 的 stride 值是最小的，所以 stride_i ≤ stride_j，即stride_i - stride_j ≤ 0
  - 调度后，stride_i 变为 stride_i + pass_i
  - 因此，stride_i + pass_i - stride_j ≤ pass_i ≤ 最大 pass 值
  - 所以在第 k+1 次调度前，i 和 j 的 stride 差值不超过最大 pass 值

  **情况3：如果 j 是进程 A，i 不是**
  - 类似情况2，可以证明 stride_i - (stride_j + pass_j) ≤ 最大 pass 值

因此，在第 k+1 次调度前，任意两个进程的 stride 差值仍然不超过最大 pass 值。

###### 4. 总结
综上，根据数学归纳法，证明了在任意时刻，系统中任意两个进程的 stride 差值不会超过最大 pass 值，即 BigStride / 2，因此有任意时刻STRIDE_MAX – STRIDE_MIN <= BigStride / 2成立，证毕。

### 3. 
已知以上结论，考虑溢出的情况下，可以为 Stride 设计特别的比较器，让 BinaryHeap<Stride> 的 pop 方法能返回真正最小的 Stride。补全下列代码中的 partial_cmp 函数，假设两个 Stride 永远不会相等。
答：
- 首先计算两个 stride 值的差的绝对值，避免溢出问题
- 如果差值大于 BigStride/2，说明可能发生了溢出，需要反转比较结果
- 否则，按照正常的大小关系比较
```rs
   use core::cmp::Ordering;

   struct Stride(u64);

   impl PartialOrd for Stride {
       fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
          // 对于 8 bits 的情况，BigStride = 255
          // 计算差值的绝对值
          let diff = self.0.abs_diff(other.0);
          // 如果差值大于 BigStride / 2，认为发生了溢出
          if diff > 255 / 2 {
              // 反转比较结果
              Some(other.0.cmp(&self.0))
          } else {
              // 正常比较
              Some(self.0.cmp(&other.0))
          }
       }
   }
   impl PartialEq for Stride {
       fn eq(&self, other: &Self) -> bool {
           false
       }
   }
```
TIPS: 使用 8 bits 存储 stride, BigStride = 255, 则: (125 < 255) == false, (129 < 255) == true.


## 荣誉准则
1. 在完成本次实验的过程（含此前学习的过程）中，我曾分别与 以下各位 就（与本次实验相关的）以下方面做过交流，还在代码中对应的位置以注释形式记录了具体的交流对象及内容：
无

2. 此外，我也参考了 以下资料 ，还在代码中对应的位置以注释形式记录了具体的参考来源及内容：
实验文档<https://learningos.cn/rCore-Tutorial-Guide-2025S/chapter5>

3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。 我清楚地知道，从以上方面获得的信息在一定程度上降低了实验难度，可能会影响起评分。

4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。 我未曾也不会向他人（含此后各届同学）复制或公开我的实验代码，我有义务妥善保管好它们。 我提交至本实验的评测系统的代码，均无意于破坏或妨碍任何计算机系统的正常运转。 我清楚地知道，以上情况均为本课程纪律所禁止，若违反，对应的实验成绩将按“-100”分计。