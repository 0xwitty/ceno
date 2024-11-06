# 0 "rv32ui/bgeu.S"
# 0 "<built-in>"
# 0 "<command-line>"
# 1 "rv32ui/bgeu.S"
# See LICENSE for license details.

# 1 "./../env/v/riscv_test.h" 1





# 1 "./../env/v/../p/riscv_test.h" 1





# 1 "./../env/v/../p/../encoding.h" 1
# 7 "./../env/v/../p/riscv_test.h" 2
# 7 "./../env/v/riscv_test.h" 2
# 4 "rv32ui/bgeu.S" 2



# 1 "rv32ui/../rv64ui/bgeu.S" 1
# See LICENSE for license details.

#*****************************************************************************
# bgeu.S
#-----------------------------------------------------------------------------

# Test bgeu instruction.



# 1 "./macros/scalar/test_macros.h" 1






#-----------------------------------------------------------------------
# Helper macros
#-----------------------------------------------------------------------
# 20 "./macros/scalar/test_macros.h"
# We use a macro hack to simpify code generation for various numbers
# of bubble cycles.
# 36 "./macros/scalar/test_macros.h"
#-----------------------------------------------------------------------
# RV64UI MACROS
#-----------------------------------------------------------------------

#-----------------------------------------------------------------------
# Tests for instructions with immediate operand
#-----------------------------------------------------------------------
# 92 "./macros/scalar/test_macros.h"
#-----------------------------------------------------------------------
# Tests for an instruction with register operands
#-----------------------------------------------------------------------
# 120 "./macros/scalar/test_macros.h"
#-----------------------------------------------------------------------
# Tests for an instruction with register-register operands
#-----------------------------------------------------------------------
# 214 "./macros/scalar/test_macros.h"
#-----------------------------------------------------------------------
# Test memory instructions
#-----------------------------------------------------------------------
# 347 "./macros/scalar/test_macros.h"
#-----------------------------------------------------------------------
# Test jump instructions
#-----------------------------------------------------------------------
# 376 "./macros/scalar/test_macros.h"
#-----------------------------------------------------------------------
# RV64UF MACROS
#-----------------------------------------------------------------------

#-----------------------------------------------------------------------
# Tests floating-point instructions
#-----------------------------------------------------------------------
# 735 "./macros/scalar/test_macros.h"
#-----------------------------------------------------------------------
# Pass and fail code (assumes test num is in gp)
#-----------------------------------------------------------------------
# 747 "./macros/scalar/test_macros.h"
#-----------------------------------------------------------------------
# Test data section
#-----------------------------------------------------------------------
# 12 "rv32ui/../rv64ui/bgeu.S" 2

.macro init; .endm
.text; .global extra_boot; extra_boot: ret; .global trap_filter; trap_filter: li a0, 0; ret; .global pf_filter; pf_filter: li a0, 0; ret; .global userstart; userstart: init

  #-------------------------------------------------------------
  # Branch tests
  #-------------------------------------------------------------

  # Each test checks both forward and backward branches

  test_2: li gp, 2; li x1, 0x00000000; li x2, 0x00000000; bgeu x1, x2, 2f; bne x0, gp, fail; 1: bne x0, gp, 3f; 2: bgeu x1, x2, 1b; bne x0, gp, fail; 3:;
  test_3: li gp, 3; li x1, 0x00000001; li x2, 0x00000001; bgeu x1, x2, 2f; bne x0, gp, fail; 1: bne x0, gp, 3f; 2: bgeu x1, x2, 1b; bne x0, gp, fail; 3:;
  test_4: li gp, 4; li x1, 0xffffffff; li x2, 0xffffffff; bgeu x1, x2, 2f; bne x0, gp, fail; 1: bne x0, gp, 3f; 2: bgeu x1, x2, 1b; bne x0, gp, fail; 3:;
  test_5: li gp, 5; li x1, 0x00000001; li x2, 0x00000000; bgeu x1, x2, 2f; bne x0, gp, fail; 1: bne x0, gp, 3f; 2: bgeu x1, x2, 1b; bne x0, gp, fail; 3:;
  test_6: li gp, 6; li x1, 0xffffffff; li x2, 0xfffffffe; bgeu x1, x2, 2f; bne x0, gp, fail; 1: bne x0, gp, 3f; 2: bgeu x1, x2, 1b; bne x0, gp, fail; 3:;
  test_7: li gp, 7; li x1, 0xffffffff; li x2, 0x00000000; bgeu x1, x2, 2f; bne x0, gp, fail; 1: bne x0, gp, 3f; 2: bgeu x1, x2, 1b; bne x0, gp, fail; 3:;

  test_8: li gp, 8; li x1, 0x00000000; li x2, 0x00000001; bgeu x1, x2, 1f; bne x0, gp, 2f; 1: bne x0, gp, fail; 2: bgeu x1, x2, 1b; 3:;
  test_9: li gp, 9; li x1, 0xfffffffe; li x2, 0xffffffff; bgeu x1, x2, 1f; bne x0, gp, 2f; 1: bne x0, gp, fail; 2: bgeu x1, x2, 1b; 3:;
  test_10: li gp, 10; li x1, 0x00000000; li x2, 0xffffffff; bgeu x1, x2, 1f; bne x0, gp, 2f; 1: bne x0, gp, fail; 2: bgeu x1, x2, 1b; 3:;
  test_11: li gp, 11; li x1, 0x7fffffff; li x2, 0x80000000; bgeu x1, x2, 1f; bne x0, gp, 2f; 1: bne x0, gp, fail; 2: bgeu x1, x2, 1b; 3:;

  #-------------------------------------------------------------
  # Bypassing tests
  #-------------------------------------------------------------

  test_12: li gp, 12; li x4, 0; 1: li x1, 0xefffffff; li x2, 0xf0000000; bgeu x1, x2, fail; addi x4, x4, 1; li x5, 2; bne x4, x5, 1b;
  test_13: li gp, 13; li x4, 0; 1: li x1, 0xefffffff; li x2, 0xf0000000; nop; bgeu x1, x2, fail; addi x4, x4, 1; li x5, 2; bne x4, x5, 1b;
  test_14: li gp, 14; li x4, 0; 1: li x1, 0xefffffff; li x2, 0xf0000000; nop; nop; bgeu x1, x2, fail; addi x4, x4, 1; li x5, 2; bne x4, x5, 1b;
  test_15: li gp, 15; li x4, 0; 1: li x1, 0xefffffff; nop; li x2, 0xf0000000; bgeu x1, x2, fail; addi x4, x4, 1; li x5, 2; bne x4, x5, 1b;
  test_16: li gp, 16; li x4, 0; 1: li x1, 0xefffffff; nop; li x2, 0xf0000000; nop; bgeu x1, x2, fail; addi x4, x4, 1; li x5, 2; bne x4, x5, 1b;
  test_17: li gp, 17; li x4, 0; 1: li x1, 0xefffffff; nop; nop; li x2, 0xf0000000; bgeu x1, x2, fail; addi x4, x4, 1; li x5, 2; bne x4, x5, 1b;

  test_18: li gp, 18; li x4, 0; 1: li x1, 0xefffffff; li x2, 0xf0000000; bgeu x1, x2, fail; addi x4, x4, 1; li x5, 2; bne x4, x5, 1b;
  test_19: li gp, 19; li x4, 0; 1: li x1, 0xefffffff; li x2, 0xf0000000; nop; bgeu x1, x2, fail; addi x4, x4, 1; li x5, 2; bne x4, x5, 1b;
  test_20: li gp, 20; li x4, 0; 1: li x1, 0xefffffff; li x2, 0xf0000000; nop; nop; bgeu x1, x2, fail; addi x4, x4, 1; li x5, 2; bne x4, x5, 1b;
  test_21: li gp, 21; li x4, 0; 1: li x1, 0xefffffff; nop; li x2, 0xf0000000; bgeu x1, x2, fail; addi x4, x4, 1; li x5, 2; bne x4, x5, 1b;
  test_22: li gp, 22; li x4, 0; 1: li x1, 0xefffffff; nop; li x2, 0xf0000000; nop; bgeu x1, x2, fail; addi x4, x4, 1; li x5, 2; bne x4, x5, 1b;
  test_23: li gp, 23; li x4, 0; 1: li x1, 0xefffffff; nop; nop; li x2, 0xf0000000; bgeu x1, x2, fail; addi x4, x4, 1; li x5, 2; bne x4, x5, 1b;

  #-------------------------------------------------------------
  # Test delay slot instructions not executed nor bypassed
  #-------------------------------------------------------------

  test_24: li gp, 24; li x1, 1; bgeu x1, x0, 1f; addi x1, x1, 1; addi x1, x1, 1; addi x1, x1, 1; addi x1, x1, 1; 1: addi x1, x1, 1; addi x1, x1, 1;; li x7, ((3) & ((1 << (32 - 1) << 1) - 1)); bne x1, x7, fail;
# 67 "rv32ui/../rv64ui/bgeu.S"
  bne x0, gp, pass; fail: sll a0, gp, 1; 1:beqz a0, 1b; or a0, a0, 1; scall;; pass: li a0, 1; scall

unimp

  .data
 .pushsection .tohost,"aw",@progbits; .align 6; .global tohost; tohost: .dword 0; .size tohost, 8; .align 6; .global fromhost; fromhost: .dword 0; .size fromhost, 8; .popsection; .align 4; .global begin_signature; begin_signature:

 


# 8 "rv32ui/bgeu.S" 2