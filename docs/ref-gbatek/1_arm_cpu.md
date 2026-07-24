## 1. ARM7TDMI Overview

- **32-bit RISC CPU**
- Designed for **high performance** and **low power consumption**

### Pipeline

```text
Fetch → Decode → Execute
```

Three-stage pipeline enables simultaneous instruction fetch, decode, and execution.

### Data Types

| Size   | Type     |
| ------ | -------- |
| 8-bit  | Byte     |
| 16-bit | Halfword |
| 32-bit | Word     |

### CPU States

| ARM State                           | THUMB State                         |
| ----------------------------------- | ----------------------------------- |
| 32-bit instructions                 | 16-bit instructions                 |
| Higher performance on 32-bit memory | Better performance on 16-bit memory |
| Access to R0–R15                    | Most instructions use R0–R7         |
| Larger code size                    | Smaller code size                   |

Both states use the same **32-bit registers** and **32-bit address space**.

### State Switching

```text
ARM ⇄ THUMB (BX instruction)
```

- Switched by the **BX** instruction.
- Both states share the same registers.

### Exceptions

```text
Exception → ARM State
Return → Previous State
```

Exceptions automatically enter **ARM state** and restore the previous state on return.
