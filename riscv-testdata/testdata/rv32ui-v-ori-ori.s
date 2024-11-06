# 0 "rv32ui/ori.S"
# 0 "<built-in>"
# 0 "<command-line>"
# 1 "rv32ui/ori.S"
# See LICENSE for license details.

# 1 "./../env/v/riscv_test.h" 1





# 1 "./../env/v/../p/riscv_test.h" 1





# 1 "./../env/v/../p/../encoding.h" 1
# 7 "./../env/v/../p/riscv_test.h" 2
# 7 "./../env/v/riscv_test.h" 2
# 4 "rv32ui/ori.S" 2



# 1 "rv32ui/../rv64ui/ori.S" 1
# See LICENSE for license details.

#*****************************************************************************
# ori.S
#-----------------------------------------------------------------------------

# Test ori instruction.



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
# 12 "rv32ui/../rv64ui/ori.S" 2

.macro init; .endm
.text; .global extra_boot; extra_boot: ret; .global trap_filter; trap_filter: li a0, 0; ret; .global pf_filter; pf_filter: li a0, 0; ret; .global userstart; userstart: init

  #-------------------------------------------------------------
  # Logical tests
  #-------------------------------------------------------------

  test_2: li gp, 2; li x13, ((0xffffffffff00ff00) & ((1 << (32 - 1) << 1) - 1)); ori x14, x13, ((0xf0f) | (-(((0xf0f) >> 11) & 1) << 11));; li x7, ((0xffffffffffffff0f) & ((1 << (32 - 1) << 1) - 1)); bne x14, x7, fail;;
  test_3: li gp, 3; li x13, ((0x000000000ff00ff0) & ((1 << (32 - 1) << 1) - 1)); ori x14, x13, ((0x0f0) | (-(((0x0f0) >> 11) & 1) << 11));; li x7, ((0x000000000ff00ff0) & ((1 << (32 - 1) << 1) - 1)); bne x14, x7, fail;;
  test_4: li gp, 4; li x13, ((0x0000000000ff00ff) & ((1 << (32 - 1) << 1) - 1)); ori x14, x13, ((0x70f) | (-(((0x70f) >> 11) & 1) << 11));; li x7, ((0x0000000000ff07ff) & ((1 << (32 - 1) << 1) - 1)); bne x14, x7, fail;;
  test_5: li gp, 5; li x13, ((0xfffffffff00ff00f) & ((1 << (32 - 1) << 1) - 1)); ori x14, x13, ((0x0f0) | (-(((0x0f0) >> 11) & 1) << 11));; li x7, ((0xfffffffff00ff0ff) & ((1 << (32 - 1) << 1) - 1)); bne x14, x7, fail;;

  #-------------------------------------------------------------
  # Source/Destination tests
  #-------------------------------------------------------------

  test_6: li gp, 6; li x11, ((0xff00ff00) & ((1 << (32 - 1) << 1) - 1)); ori x11, x11, ((0x0f0) | (-(((0x0f0) >> 11) & 1) << 11));; li x7, ((0xff00fff0) & ((1 << (32 - 1) << 1) - 1)); bne x11, x7, fail;;

  #-------------------------------------------------------------
  # Bypassing tests
  #-------------------------------------------------------------

  test_7: li gp, 7; li x4, 0; 1: li x1, ((0x000000000ff00ff0) & ((1 << (32 - 1) << 1) - 1)); ori x14, x1, ((0x0f0) | (-(((0x0f0) >> 11) & 1) << 11)); addi x6, x14, 0; addi x4, x4, 1; li x5, 2; bne x4, x5, 1b; li x7, ((0x000000000ff00ff0) & ((1 << (32 - 1) << 1) - 1)); bne x6, x7, fail;;
  test_8: li gp, 8; li x4, 0; 1: li x1, ((0x0000000000ff00ff) & ((1 << (32 - 1) << 1) - 1)); ori x14, x1, ((0x70f) | (-(((0x70f) >> 11) & 1) << 11)); nop; addi x6, x14, 0; addi x4, x4, 1; li x5, 2; bne x4, x5, 1b; li x7, ((0x0000000000ff07ff) & ((1 << (32 - 1) << 1) - 1)); bne x6, x7, fail;;
  test_9: li gp, 9; li x4, 0; 1: li x1, ((0xfffffffff00ff00f) & ((1 << (32 - 1) << 1) - 1)); ori x14, x1, ((0x0f0) | (-(((0x0f0) >> 11) & 1) << 11)); nop; nop; addi x6, x14, 0; addi x4, x4, 1; li x5, 2; bne x4, x5, 1b; li x7, ((0xfffffffff00ff0ff) & ((1 << (32 - 1) << 1) - 1)); bne x6, x7, fail;;

  test_10: li gp, 10; li x4, 0; 1: li x1, ((0x000000000ff00ff0) & ((1 << (32 - 1) << 1) - 1)); ori x14, x1, ((0x0f0) | (-(((0x0f0) >> 11) & 1) << 11)); addi x4, x4, 1; li x5, 2; bne x4, x5, 1b; li x7, ((0x000000000ff00ff0) & ((1 << (32 - 1) << 1) - 1)); bne x14, x7, fail;;
  test_11: li gp, 11; li x4, 0; 1: li x1, ((0x0000000000ff00ff) & ((1 << (32 - 1) << 1) - 1)); nop; ori x14, x1, ((0xf0f) | (-(((0xf0f) >> 11) & 1) << 11)); addi x4, x4, 1; li x5, 2; bne x4, x5, 1b; li x7, ((0xffffffffffffffff) & ((1 << (32 - 1) << 1) - 1)); bne x14, x7, fail;;
  test_12: li gp, 12; li x4, 0; 1: li x1, ((0xfffffffff00ff00f) & ((1 << (32 - 1) << 1) - 1)); nop; nop; ori x14, x1, ((0x0f0) | (-(((0x0f0) >> 11) & 1) << 11)); addi x4, x4, 1; li x5, 2; bne x4, x5, 1b; li x7, ((0xfffffffff00ff0ff) & ((1 << (32 - 1) << 1) - 1)); bne x14, x7, fail;;

  test_13: li gp, 13; ori x1, x0, ((0x0f0) | (-(((0x0f0) >> 11) & 1) << 11));; li x7, ((0x0f0) & ((1 << (32 - 1) << 1) - 1)); bne x1, x7, fail;;
  test_14: li gp, 14; li x1, ((0x00ff00ff) & ((1 << (32 - 1) << 1) - 1)); ori x0, x1, ((0x70f) | (-(((0x70f) >> 11) & 1) << 11));; li x7, ((0) & ((1 << (32 - 1) << 1) - 1)); bne x0, x7, fail;;

  bne x0, gp, pass; fail: sll a0, gp, 1; 1:beqz a0, 1b; or a0, a0, 1; scall;; pass: li a0, 1; scall

unimp

  .data
 .pushsection .tohost,"aw",@progbits; .align 6; .global tohost; tohost: .dword 0; .size tohost, 8; .align 6; .global fromhost; fromhost: .dword 0; .size fromhost, 8; .popsection; .align 4; .global begin_signature; begin_signature:

 


# 8 "rv32ui/ori.S" 2