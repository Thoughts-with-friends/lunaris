// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! cpuinstrs.hpp
//!

/// ARM instruction enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ARMInstr {
    Undefined,
    DataProcessing,
    CountLeadingZeros,
    SaturatedOp, // QADD, etc
    Multiply,
    MultiplyLong,
    SignedHalfwordMultiply,
    Swap,
    Branch,
    BranchWithLink,
    BranchExchange,
    BranchLinkExchange,
    StoreHalfword,
    LoadHalfword,
    LoadDoubleword,
    LoadSignedByte,
    StoreDoubleword,
    LoadSignedHalfword,
    StoreWord,
    LoadWord,
    StoreByte,
    LoadByte,
    StoreBlock,
    LoadBlock,
    CopRegTransfer,
    CopDataOp,
    Swi,
}

/// Thumb instruction enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbInstr {
    Undefined,
    MovShift,
    AddReg,
    SubReg,
    MovImm,
    CmpImm,
    AddImm,
    SubImm,
    AluOp,
    HiRegOp,
    PcRelLoad,
    StoreRegOffset,
    LoadRegOffset,
    LoadStoreSignHalfword,
    StoreHalfword,
    LoadHalfword,
    StoreImmOffset,
    LoadImmOffset,
    SpRelStore,
    SpRelLoad,
    OffsetSp,
    LoadAddress,
    Pop,
    Push,
    StoreMultiple,
    LoadMultiple,
    Branch,
    CondBranch,
    LongBranchPrep,
    LongBranch,
    LongBlx,
    #[expect(unused)]
    Swi,
}
