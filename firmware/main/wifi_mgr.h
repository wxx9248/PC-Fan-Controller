#pragma once

#include <stdbool.h>
#include <stdint.h>

// Starts WiFi STA + mDNS. Returns true once WiFi has started (it may still be
// connecting), and false when WiFi is disabled: either no SSID is configured, or
// one of the steps that bring it up failed, which is logged on UART0.
//
// The return value matters: when it is false the TCP/IP stack may never have
// been initialised (esp_netif_init() is the only caller of lwIP's tcpip_init()),
// so anything that opens a socket, such as the HTTP server, must not be started.
// Doing so asserts inside lwIP and panics, which is a reboot loop.
bool wifi_mgr_start(void);
// Returns true and fills *rssi_dbm when associated with an AP.
bool wifi_mgr_get_rssi(int8_t *rssi_dbm);
