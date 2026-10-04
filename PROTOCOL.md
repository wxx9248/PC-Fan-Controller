# Serial Control Protocol

Line-based ASCII over the ESP32-C3's native USB Serial/JTAG port (appears on
Linux as a CDC-ACM device, `/dev/ttyACM*`; use the immutable
`/dev/serial/by-id/usb-Espressif_USB_JTAG_serial_debug_unit_<serial>-if00`
path). Baud rate is ignored (native USB). The firmware routes all logging to
UART0, so this channel carries only protocol traffic.

## Framing

- Commands and replies are single lines terminated by `\n` (`\r` is ignored).
  Note for interactive terminals: the Return key sends `\r`, not `\n`, so a
  terminal program must be told to translate it (for picocom, `--omap crlf`) or
  the firmware will never see a line terminator at all.
- Commands are uppercase. Maximum command length: 63 bytes; a longer line is
  rejected with `ERR line too long` and has no effect.
- Every line gets at most one reply line, an empty line included. If the
  transmit buffer stays full for 100 ms because the host is not reading, the
  firmware drops that reply, so a host must treat a missing reply as a timeout.
- Each reply is written in a single operation, so a reply is never split across
  its newline even when the host is not draining the port.

## Commands

| Command      | Reply                                             | Effect |
|--------------|---------------------------------------------------|--------|
| `PWM <0-100>`| `OK` or `ERR PWM expects an integer 0-100`        | Sets fan duty (% as seen by the fan). This is the only command that resets the failsafe timer, and it also clears the failsafe flag. A bare `PWM` with no argument gets the same `ERR`. |
| `PING`       | `PONG`                                            | Liveness probe. Does not reset the failsafe timer. |
| `STATUS`     | `STATUS PWM=<0-100> RPM=<n> FAILSAFE=<0\|1> UPTIME=<seconds> FAILSAFE_TIMEOUT=<seconds>` | Reports current state. `FAILSAFE_TIMEOUT` is the firmware's failsafe timeout (`CONFIG_FAN_FAILSAFE_TIMEOUT_S`). Hosts must ignore fields they do not recognise, which is what lets a minor version add one. |
| `VER`        | `VER <semver>`                                    | Firmware protocol version. |
| (empty line) | `ERR empty command`                               | |
| (over 63 chars) | `ERR line too long`                            | The line is discarded; truncation is never silently accepted. |
| anything else| `ERR unknown command`                             | |

## Failsafe semantics

- At boot the fan runs at `CONFIG_FAN_BOOT_DUTY_PERCENT` (default 100%) and
  `FAILSAFE=1` until the first `PWM` command.
- If no `PWM` command arrives for `CONFIG_FAN_FAILSAFE_TIMEOUT_S` (default
  30 s), the fan is forced to 100% and `FAILSAFE=1` until the next `PWM`.
  `STATUS`, `VER` and `PING` do not reset the timer. That is deliberate: they
  prove the link is alive without proving the host is still *controlling* the
  fan, so a host whose control loop had died could otherwise hold the fan at a
  low duty indefinitely just by polling.
- Consequence for hosts: send `PWM` at an interval of at most half the failsafe
  timeout, even when the duty is unchanged. Budget for whatever else happens
  between two `PWM` commands, not only the sleep. `fan-controllerd` issues a
  `STATUS` exchange in that gap, so it validates
  `poll_interval + serial_timeout <= failsafe / 2` rather than the interval alone.
  The firmware reports the timeout in `STATUS`, so a host can check its
  cadence against the real value instead of a configured copy; `fan-controllerd`
  does that each time it connects.

## Example session

```
> VER
< VER 1.0.0
> PWM 45
< OK
> STATUS
< STATUS PWM=45 RPM=1110 FAILSAFE=0 UPTIME=3600 FAILSAFE_TIMEOUT=30
> PWM 145
< ERR PWM expects an integer 0-100
> PING
< PONG
```

## HTTP API

A second, strictly read-only interface, served on port 80 when WiFi is up.
Control belongs exclusively to the USB host: no HTTP request can change the fan
speed. Three implementations have to agree on this shape. The firmware produces
it (`firmware/main/http_server.c`), the status page consumes it, and the
development mock reimplements it (`frontend/mock/simulation.mjs`). This section is
encoded once, in `frontend/src/lib/status.js`: the page checks every reply with it
before drawing anything, and the mock's tests check the mock against it.

### `GET /api/status`

`200 OK`, `Content-Type: application/json`, a flat object with exactly these
five fields:

| Field | JSON type | Range / meaning |
|---|---|---|
| `pwmPercent` | integer | `0`-`100`. Duty as the fan sees it, after the inverting transistor stage is compensated for. |
| `rpm` | integer | Always a multiple of 30. The tachometer counts pulses over a 1 s window at 2 pulses per revolution, so 30 RPM is its resolution. |
| `failsafe` | boolean | `true` while the host is not in control: from boot until the first `PWM`, and again after the failsafe timeout expires. |
| `uptimeSeconds` | integer | Seconds since boot. Not wall-clock time; it resets on reboot. |
| `wifiRssiDbm` | integer or `null` | Signal strength when associated, `null` when not. Consumers must handle the null. |

No field is ever absent. There is no `Access-Control-Allow-Origin` header,
because the page is served from the same origin as the API. (The development mock
does send one, since there the page comes from Vite on a different port.)

### `GET /`

`200 OK`, `Content-Type: text/html`, `Content-Encoding: gzip`. The body is always
gzip-compressed: the page is stored compressed and served verbatim, so no
identity encoding is available. gzip rather than br or zstd because browsers only
accept those over HTTPS, and this device serves plain HTTP.

### Errors

| Condition | Response |
|---|---|
| A method other than `GET` on `/` or `/api/status` | `405 Method Not Allowed` |
| Any other path, whatever the method | `404 Not Found` |
| A request the HTTP parser rejects, such as an unknown method | `400 Bad Request` |

All three come from `esp_http_server` itself: a short plain-text message, sent
as `Content-Type: text/html`. Error bodies are never JSON. The development mock
serves only `/api/status` and answers with the same codes, with JSON bodies.

## Version compatibility

`VER` reports `<major>.<minor>.<patch>`. A host should refuse to drive firmware
whose major version differs from the one it was written against, since the
framing or the meaning of commands may have changed. `fan-controllerd` enforces
this. A minor version may only add, such as a new `STATUS` field, which a host
written for the earlier version ignores.
