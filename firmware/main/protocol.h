#pragma once

// The host's control channel: the line protocol over USB Serial/JTAG
// (PROTOCOL.md). Start it after failsafe_start().
void protocol_start(void);
