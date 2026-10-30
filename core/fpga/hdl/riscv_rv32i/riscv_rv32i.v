// Sample 2: a RISC-V RV32I CPU as a Slot-2 co-processor.
//
// Single-cycle, Harvard: 4 KiB instruction memory and 4 KiB data memory,
// both loaded / inspected by the DS through the bus. All of RV32I except
// FENCE (a no-op) and CSRs; ECALL / EBREAK (and illegal opcodes) halt.
//
//  DS (ARM9) bus window 0x0800_0000 + ...      RISC-V address space
//    +0x0000 CTRL      bit0 run, bit1 reset core   0x0000_0000.. DMEM (addr[11:2])
//    +0x0004 STATUS    bit0 running, bit1 halted,  0x8000_0000  MAILBOX_OUT (store)
//                      bit2 illegal instruction    0x8000_0004  MAILBOX_IN  (load)
//    +0x0008 PC        +0x000C INSTRET
//    +0x0010 MAILBOX_OUT (CPU -> DS)   +0x0014 MAILBOX_IN (DS -> CPU)
//    +0x0018 IRQ_EN    +0x001C IRQ_STATUS (bit0 halted, bit1 mailbox; w1c)
//    +0x1000 IMEM (1024 words)         +0x2000 DMEM (1024 words)
//
// The DS loads a program into IMEM, sets CTRL = 3 (reset + run), and gets
// an interrupt when the program halts or writes the mailbox.
//
// ds_fpga: size=0x4000 clock_div=1024

`default_nettype none

module riscv_rv32i (
    input  wire        clk,
    input  wire        rst,
    input  wire        bus_sel,
    input  wire        bus_we,
    input  wire [15:0] bus_addr,
    input  wire [3:0]  bus_be,
    input  wire [31:0] bus_wdata,
    output reg  [31:0] bus_rdata,
    output wire        irq
);
    localparam [15:0] ADDR_CTRL        = 16'h0000; // bit0 run, bit1 reset core (pc = 0)
    localparam [15:0] ADDR_STATUS      = 16'h0004; // bit0 running, bit1 halted, bit2 illegal
    localparam [15:0] ADDR_PC          = 16'h0008; // program counter
    localparam [15:0] ADDR_INSTRET     = 16'h000C; // instructions retired
    localparam [15:0] ADDR_MAILBOX_OUT = 16'h0010; // written by the CPU (store to 0x80000000)
    localparam [15:0] ADDR_MAILBOX_IN  = 16'h0014; // read by the CPU (load from 0x80000004)
    localparam [15:0] ADDR_IRQ_EN      = 16'h0018; // bit0 halt, bit1 mailbox
    localparam [15:0] ADDR_IRQ_STATUS  = 16'h001C; // bit0 halted, bit1 mailbox written (w1c)
    localparam [15:0] ADDR_IMEM        = 16'h1000; // instruction memory, 1024 words
    localparam [15:0] ADDR_DMEM        = 16'h2000; // data memory, 1024 words

    // ---- state ----
    reg [31:0] imem [0:1023];
    reg [31:0] dmem [0:1023];
    reg [31:0] x    [0:31];
    reg [31:0] pc;
    reg [31:0] instret;
    reg        running;
    reg        halted;
    reg        illegal_seen;
    reg [31:0] mailbox_out;
    reg [31:0] mailbox_in;
    reg [1:0]  irq_en;
    reg [1:0]  irq_status;

    // ---- decode ----
    wire [31:0] instr  = imem[pc[11:2]];
    wire [6:0]  opcode = instr[6:0];
    wire [4:0]  rd     = instr[11:7];
    wire [2:0]  f3     = instr[14:12];
    wire [4:0]  rs1    = instr[19:15];
    wire [4:0]  rs2    = instr[24:20];
    wire        f7_5   = instr[30];

    wire [31:0] v1 = (rs1 == 5'd0) ? 32'd0 : x[rs1];
    wire [31:0] v2 = (rs2 == 5'd0) ? 32'd0 : x[rs2];

    wire [31:0] imm_i = {{20{instr[31]}}, instr[31:20]};
    wire [31:0] imm_s = {{20{instr[31]}}, instr[31:25], instr[11:7]};
    wire [31:0] imm_b = {{19{instr[31]}}, instr[31], instr[7], instr[30:25], instr[11:8], 1'b0};
    wire [31:0] imm_u = {instr[31:12], 12'd0};
    wire [31:0] imm_j = {{11{instr[31]}}, instr[31], instr[19:12], instr[20], instr[30:21], 1'b0};

    localparam [6:0] OP_LUI    = 7'h37;
    localparam [6:0] OP_AUIPC  = 7'h17;
    localparam [6:0] OP_JAL    = 7'h6F;
    localparam [6:0] OP_JALR   = 7'h67;
    localparam [6:0] OP_BRANCH = 7'h63;
    localparam [6:0] OP_LOAD   = 7'h03;
    localparam [6:0] OP_STORE  = 7'h23;
    localparam [6:0] OP_IMM    = 7'h13;
    localparam [6:0] OP_REG    = 7'h33;
    localparam [6:0] OP_FENCE  = 7'h0F;
    localparam [6:0] OP_SYSTEM = 7'h73;

    wire known = (opcode == OP_LUI) | (opcode == OP_AUIPC) | (opcode == OP_JAL) | (opcode == OP_JALR)
               | (opcode == OP_BRANCH) | (opcode == OP_LOAD) | (opcode == OP_STORE) | (opcode == OP_IMM)
               | (opcode == OP_REG) | (opcode == OP_FENCE);

    // ---- ALU (OP and OP-IMM) ----
    wire [31:0] alu_b = (opcode == OP_REG) ? v2 : imm_i;
    wire [4:0]  shamt = alu_b[4:0];
    reg  [31:0] alu;
    always @(*) begin
        case (f3)
            3'd0: alu = ((opcode == OP_REG) & f7_5) ? v1 - alu_b : v1 + alu_b;
            3'd1: alu = v1 << shamt;
            3'd2: alu = ($signed(v1) < $signed(alu_b)) ? 32'd1 : 32'd0;
            3'd3: alu = (v1 < alu_b) ? 32'd1 : 32'd0;
            3'd4: alu = v1 ^ alu_b;
            3'd5: alu = f7_5 ? ($signed(v1) >>> shamt) : (v1 >> shamt);
            3'd6: alu = v1 | alu_b;
            default: alu = v1 & alu_b;
        endcase
    end

    // ---- branches ----
    reg take;
    always @(*) begin
        case (f3)
            3'd0: take = v1 == v2;
            3'd1: take = v1 != v2;
            3'd4: take = $signed(v1) < $signed(v2);
            3'd5: take = $signed(v1) >= $signed(v2);
            3'd6: take = v1 < v2;
            3'd7: take = v1 >= v2;
            default: take = 1'b0;
        endcase
    end

    // ---- loads ----
    wire [31:0] ld_addr  = v1 + imm_i;
    wire [31:0] ld_word  = ld_addr[31] ? (ld_addr[2] ? mailbox_in : mailbox_out) : dmem[ld_addr[11:2]];
    wire [31:0] ld_shift = ld_word >> {ld_addr[1:0], 3'b000};
    reg  [31:0] ld_val;
    always @(*) begin
        case (f3)
            3'd0: ld_val = {{24{ld_shift[7]}}, ld_shift[7:0]};
            3'd1: ld_val = {{16{ld_shift[15]}}, ld_shift[15:0]};
            3'd4: ld_val = {24'd0, ld_shift[7:0]};
            3'd5: ld_val = {16'd0, ld_shift[15:0]};
            default: ld_val = ld_word;
        endcase
    end

    // ---- stores ----
    wire [31:0] st_addr = v1 + imm_s;
    wire [31:0] st_data = v2 << {st_addr[1:0], 3'b000};
    reg  [31:0] st_mask;
    always @(*) begin
        case (f3)
            3'd0: st_mask = 32'h000000FF << {st_addr[1:0], 3'b000};
            3'd1: st_mask = 32'h0000FFFF << {st_addr[1], 4'b0000};
            default: st_mask = 32'hFFFFFFFF;
        endcase
    end

    // ---- next pc / write-back ----
    reg [31:0] next_pc;
    reg [31:0] wb;
    reg        wb_en;
    always @(*) begin
        next_pc = pc + 32'd4;
        wb = 32'd0;
        wb_en = 1'b0;
        case (opcode)
            OP_LUI:    begin wb = imm_u; wb_en = 1'b1; end
            OP_AUIPC:  begin wb = pc + imm_u; wb_en = 1'b1; end
            OP_JAL:    begin wb = pc + 32'd4; wb_en = 1'b1; next_pc = pc + imm_j; end
            OP_JALR:   begin wb = pc + 32'd4; wb_en = 1'b1; next_pc = (v1 + imm_i) & 32'hFFFFFFFE; end
            OP_BRANCH: if (take) next_pc = pc + imm_b;
            OP_LOAD:   begin wb = ld_val; wb_en = 1'b1; end
            OP_IMM:    begin wb = alu; wb_en = 1'b1; end
            OP_REG:    begin wb = alu; wb_en = 1'b1; end
            default:   ;
        endcase
    end

    wire step      = running & ~halted;
    wire stop      = step & ((opcode == OP_SYSTEM) | ~known);
    wire exec      = step & ~stop;
    wire store     = exec & (opcode == OP_STORE);
    wire mbox_set  = store & st_addr[31] & ~st_addr[2];

    // ---- host side ----
    wire       hw       = bus_sel & bus_we;
    wire [9:0] word     = bus_addr[11:2];
    wire [31:0] wmask   = {{8{bus_be[3]}}, {8{bus_be[2]}}, {8{bus_be[1]}}, {8{bus_be[0]}}};
    wire       wr_ctrl  = hw & (bus_addr == ADDR_CTRL);
    wire [1:0] ack      = (hw & (bus_addr == ADDR_IRQ_STATUS)) ? bus_wdata[1:0] : 2'b00;

    always @(posedge clk) begin
        if (rst) begin
            pc           <= 32'd0;
            instret      <= 32'd0;
            running      <= 1'b0;
            halted       <= 1'b0;
            illegal_seen <= 1'b0;
            mailbox_out  <= 32'd0;
            mailbox_in   <= 32'd0;
            irq_en       <= 2'b00;
            irq_status   <= 2'b00;
        end else begin
            // CPU
            if (exec) begin
                pc      <= next_pc;
                instret <= instret + 32'd1;
                if (wb_en & (rd != 5'd0))
                    x[rd] <= wb;
                if (store & ~st_addr[31])
                    dmem[st_addr[11:2]] <= (dmem[st_addr[11:2]] & ~st_mask) | (st_data & st_mask);
                if (mbox_set)
                    mailbox_out <= v2;
            end
            if (stop) begin
                halted       <= 1'b1;
                illegal_seen <= ~known & (opcode != OP_SYSTEM);
            end
            irq_status <= (irq_status & ~ack) | {mbox_set, stop};

            // DS writes come last, so they win over the CPU in the same cycle.
            if (wr_ctrl) begin
                running <= bus_wdata[0];
                if (bus_wdata[1]) begin
                    pc           <= 32'd0;
                    instret      <= 32'd0;
                    halted       <= 1'b0;
                    illegal_seen <= 1'b0;
                end
            end
            if (hw & (bus_addr == ADDR_MAILBOX_IN))
                mailbox_in <= (mailbox_in & ~wmask) | (bus_wdata & wmask);
            if (hw & (bus_addr == ADDR_IRQ_EN))
                irq_en <= bus_wdata[1:0];
            if (hw & (bus_addr[15:12] == 4'h1))
                imem[word] <= (imem[word] & ~wmask) | (bus_wdata & wmask);
            if (hw & (bus_addr[15:12] == 4'h2))
                dmem[word] <= (dmem[word] & ~wmask) | (bus_wdata & wmask);
        end
    end

    // ---- bus reads ----
    always @(*) begin
        if (bus_addr[15:12] == 4'h1)
            bus_rdata = imem[word];
        else if (bus_addr[15:12] == 4'h2)
            bus_rdata = dmem[word];
        else begin
            case (bus_addr)
                ADDR_CTRL:        bus_rdata = {31'd0, running};
                ADDR_STATUS:      bus_rdata = {29'd0, illegal_seen, halted, step};
                ADDR_PC:          bus_rdata = pc;
                ADDR_INSTRET:     bus_rdata = instret;
                ADDR_MAILBOX_OUT: bus_rdata = mailbox_out;
                ADDR_MAILBOX_IN:  bus_rdata = mailbox_in;
                ADDR_IRQ_EN:      bus_rdata = {30'd0, irq_en};
                ADDR_IRQ_STATUS:  bus_rdata = {30'd0, irq_status};
                default:          bus_rdata = 32'd0;
            endcase
        end
    end

    assign irq = |(irq_status & irq_en);
endmodule

`default_nettype wire
