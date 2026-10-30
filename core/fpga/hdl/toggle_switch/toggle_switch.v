// Sample 1: a toggle switch (push-on / push-off) with an LED.
//
// Every press of the switch flips the LED, counts the press and raises an
// interrupt. The DS (or the designer's FPGA panel) presses the switch by
// writing 1 to SWITCH:
//
//     *(volatile u32 *)0x08000004 = 1;              // press
//     led = *(volatile u32 *)0x08000008 & 1;        // read the LED
//
// ds_fpga: size=0x100 clock_div=1

`default_nettype none

module toggle_switch (
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
    localparam [15:0] ADDR_ID         = 16'h0000; // reads 'TGSW' (0x54475357)
    localparam [15:0] ADDR_SWITCH     = 16'h0004; // write 1 = press the switch
    localparam [15:0] ADDR_LED        = 16'h0008; // bit0: LED (also writable)
    localparam [15:0] ADDR_PRESSES    = 16'h000C; // number of presses
    localparam [15:0] ADDR_IRQ_EN     = 16'h0010; // bit0: interrupt on toggle
    localparam [15:0] ADDR_IRQ_STATUS = 16'h0014; // bit0: toggled (write 1 to clear)

    reg        led;
    reg [31:0] presses;
    reg        irq_en;
    reg        toggled;

    wire write   = bus_sel & bus_we & bus_be[0];
    wire press   = write & (bus_addr == ADDR_SWITCH) & bus_wdata[0];
    wire set_led = write & (bus_addr == ADDR_LED);
    wire ack     = write & (bus_addr == ADDR_IRQ_STATUS) & bus_wdata[0];

    always @(posedge clk) begin
        if (rst) begin
            led     <= 1'b0;
            presses <= 32'd0;
            irq_en  <= 1'b0;
            toggled <= 1'b0;
        end else begin
            if (press) begin
                led     <= ~led;
                presses <= presses + 32'd1;
            end else if (set_led) begin
                led <= bus_wdata[0];
            end
            if (write & (bus_addr == ADDR_IRQ_EN))
                irq_en <= bus_wdata[0];
            // A press in the same cycle as the acknowledge wins.
            toggled <= press | (toggled & ~ack);
        end
    end

    always @(*) begin
        case (bus_addr)
            ADDR_ID:         bus_rdata = 32'h54475357;
            ADDR_LED:        bus_rdata = {31'd0, led};
            ADDR_PRESSES:    bus_rdata = presses;
            ADDR_IRQ_EN:     bus_rdata = {31'd0, irq_en};
            ADDR_IRQ_STATUS: bus_rdata = {31'd0, toggled};
            default:         bus_rdata = 32'd0;
        endcase
    end

    assign irq = toggled & irq_en;
endmodule

`default_nettype wire
