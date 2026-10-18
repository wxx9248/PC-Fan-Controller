# PC Fan Controller

A PC fan controller for a NAS hard-drive array: an ESP32-C3 Super Mini drives
the fan's PWM line and reads its tachometer; a daemon on the NAS reads drive
temperatures and commands fan speed over USB serial. The controller also
joins WiFi and serves a read-only status page over HTTP.

```
┌─────────── NAS (Linux) ───────────┐        ┌───── ESP32-C3 Super Mini ─────┐
│ drivetemp hwmon (sysfs)           │  USB   │ firmware/ (ESP-IDF)           │
│   → fan-controllerd (daemon/)     ├────────┤   PWM 25 kHz → GPIO3 → 2N3904 │──→ fan PWM
│      temps → curve → "PWM <n>"    │ serial │   tach ← GPIO4 (10k pull-up)  │←── fan tach
└───────────────────────────────────┘        │   WiFi + HTTP status (:80)    │
                                             └───────────────────────────────┘
```

Safety model: the fan runs at 100% from power-up until the daemon takes over,
and the firmware forces 100% whenever the host goes silent for 30 s (daemon
crash, NAS down, cable out). See [PROTOCOL.md](PROTOCOL.md).

## Wiring

| ESP32-C3 pin | Connection |
|---|---|
| GPIO3 | → 10 kΩ → 2N3904 base; collector → fan PWM pin, emitter → GND (signal inverted by the transistor; compensated in firmware) |
| GPIO4 | ← fan tach pin, with external 10 kΩ pull-up to 3.3 V |

To re-verify the wiring, drive the controller directly over USB (see
[Verifying end to end](#verifying-end-to-end) for the exact command) and watch
the tachometer respond: `PWM 30` then `PWM 100`, with `STATUS` in between. RPM
should rise with duty and settle within a few seconds. Note that this fan floors
at its minimum speed below about 32% duty, so the two points must be far apart to
show a difference.

This checks the wiring *through* this firmware. To test the hardware
independently of it you would flash a minimal standalone sketch instead; no such
config is kept in this repository.

## Building & flashing the firmware

Built and tested with ESP-IDF 6.1, which is what `dependencies.lock` records (as
`6.1.0`) and what CI pins. The component manifest declares a floor of 5.3, the
release that introduced the `esp_driver_gpio` and `esp_driver_ledc` components
this firmware needs. Nothing below 6.1 has actually been built here, though, and
building on an older version makes the component manager re-solve and rewrite the
committed `dependencies.lock`. You also need Node.js with [pnpm](https://pnpm.io/) for the
status page.

```sh
# 1. Build the web frontend (embedded into the firmware image)
cd frontend
pnpm install && pnpm build        # writes firmware/main/web/index.html.gz

# 2. WiFi credentials (optional; firmware runs USB-only without them)
cd ../firmware
cp sdkconfig.secrets.example sdkconfig.secrets
$EDITOR sdkconfig.secrets

# 3. Build and flash over USB
idf.py set-target esp32c3
idf.py build flash
```

Configuration knobs (failsafe timeout, boot duty, mDNS hostname) live under
`idf.py menuconfig` → *Fan Controller Configuration*, with their defaults in
`main/Kconfig.projbuild`. menuconfig writes its changes to `sdkconfig`, which is
never committed, so a value you want to keep belongs in `sdkconfig.defaults`.
Never commit `sdkconfig` or `sdkconfig.secrets`.

Once `sdkconfig` exists, it takes precedence over `sdkconfig.secrets`. After
creating, changing or deleting the secrets file, delete `sdkconfig` and rebuild;
the build compares the two and stops with that instruction when they differ. A
built image, like `sdkconfig`, holds the WiFi password in plaintext, and the
board has no flash encryption, so anyone who can reach its USB port can read the
password back with esptool. Treat both as secrets.

The firmware build also refuses to embed a status page older than its sources,
so run `pnpm build` again after changing anything under `frontend/`.

Once on WiFi, the status page is at `http://fan-controller.local/` and JSON at
`http://fan-controller.local/api/status`. HTTP is strictly read-only. The
endpoints and the exact JSON shape are specified in
[PROTOCOL.md](PROTOCOL.md#http-api).

### Frontend development

`pnpm dev` starts Vite together with a mock controller API
(`frontend/mock/`) that simulates PWM/RPM activity, including
occasional failsafe episodes; both shut down together. The API base comes
from Vite env files: `.env.development` points at the mock
(`http://localhost:8787/api`), `.env.production` bakes the same-origin
relative path (`/api`) into the built bundle. `pnpm build` refuses to embed a
bundle that still refers to localhost.

`pnpm test` runs the mock's tests. They check its fan model against the
measured sweep in `daemon/tests/fixtures/`, and its HTTP replies against
`src/lib/status.js`, the same contract check the page applies to every reply. If
port 8787 is taken, the mock says so and the dev server carries on without it.

## Installing the daemon on the NAS

```sh
cd daemon
make CONTROLLER_SERIAL=AA:BB:CC:DD:EE:FF DISKS='["ata-MODEL_SERIAL"]'
sudo make install          # copies into place (see `make show`); never builds
sudo make install-udev     # optional /dev/fan-controller symlink
```

Build as yourself. Every build step refuses to run as root, because a root build
leaves root-owned files in `target/` and `build/` that break the next ordinary
one, and `make install` only copies what the build produced. It stops, and
tells you to run `make`, if anything is missing or older than its sources.

A machine's own values go on the `make` command line, so they are never written
into a committed file. `CONTROLLER_SERIAL` is the controller's factory MAC,
visible in its `/dev/serial/by-id/` name, and it fills in both the `device` path
in the config and the udev rule's match. `DISKS` lists the drives to monitor.
Any other value from `config/targets/common.conf` can be given the same way,
except the install layout, which belongs to the target. Give them to `make`, not
to `make install`: install never renders, and it refuses values that differ from
the ones the build was rendered with.

`make install` will not overwrite an existing `/etc/fan-controllerd.toml`; it
says so and leaves the freshly rendered copy in `build/local/` for you to diff.

### How the config files are produced

Nothing under `daemon/config/` is a finished file. `config/templates/*.in` hold
the text, `config/targets/*.conf` hold the values, and
`scripts/render-config` combines them into `build/<target>/`:

| Target | Binary | Unit | udev rule | modules-load |
|---|---|---|---|---|
| `local` | `/usr/local/bin` | `/etc/systemd/system` | `/etc/udev/rules.d` | `/etc/modules-load.d` |
| `debian` | `/usr/bin` | `/usr/lib/systemd/system` | `/usr/lib/udev/rules.d` | `/usr/lib/modules-load.d` |

`/etc/fan-controllerd.toml` is the config path for both, because that is what the
daemon compiles in as its default.

`config/targets/common.conf` declares every variable. It gives each one a
default except the install layout, which it leaves empty so that every target
states its own; a target that misses one fails the render. A target file sets
only what it does differently, so each value is written in one place, and the
renderer enforces it: a template referring to an undeclared name, or a target
setting one, fails the render instead of shipping a file with an unsubstituted
placeholder. `make targets` lists the targets and
`make show TARGET=…` prints the resolved layout. Supporting another distribution
is one more file in `config/targets/`.

The rendered config for every target is parsed and run through the daemon's own
validator by `cargo test`, so a template and the code cannot drift apart.

Temperature source. The daemon reads drive temperatures only from the kernel
`drivetemp` hwmon, which is a plain sysfs read needing no privilege and no
subprocess. That makes the module a hard requirement. `make install` and the
Debian package both install a `modules-load.d` snippet that loads it at boot, so
it only has to be loaded by hand once, before the first reboot:

```sh
sudo modprobe drivetemp
```

`drivetemp` binds to native ATA/SATA drives. It cannot bind to a drive behind a
USB bridge, so those drives cannot be monitored, which is fine here because they
sit outside the fan's airflow. NVMe drives cannot be monitored either. The
kernel's `nvme` driver does publish a temperature, but its hwmon sits directly
under the NVMe controller instead of in the `hwmon/` directory the daemon reads,
so a listed NVMe drive reads as unreadable and holds the fan at 100%, with an
error that wrongly suggests loading `drivetemp`.

There is deliberately no `smartctl` fallback: it would need root, it spawns a
process per disk per cycle with no timeout, and its exit status cannot tell
"drive asleep" apart from "drive unreadable", so an unreadable drive would be
treated as one needing only minimum cooling.

Each poll reads every listed drive, and the kernel's `drivetemp` documentation
notes that reading the temperature can reset some drives' spin-down timer. A
drive set to spin down may then never do so.

Edit `/etc/fan-controllerd.toml`:

- `device`: the controller's immutable path. Pick yours from
  `ls /dev/serial/by-id/` (contains the chip's factory MAC; stable across
  USB ports, replugs, and other ttyACM devices). It is already filled in if you
  gave `CONTROLLER_SERIAL` to `make`.
- `disks`: immutable names from `ls /dev/disk/by-id/` (prefer
  `ata-<MODEL>_<SERIAL>` or `wwn-0x…`; these are independent of `sdX`
  numbering and SATA port). Native SATA only, as above.
- `curve`: `[°C, PWM%]` points. The hottest monitored drive drives the curve.
  Temperatures come from the kernel `drivetemp` hwmon and nothing else, as above.
  A drive listed here whose temperature cannot be read is an error, not a
  fallback: the daemon commands 100%.
- `fan_floor_duty_percent`: the top of your fan's dead zone, meaning the highest
  duty that still yields its minimum speed. Diagnostic only; it never rescales
  the curve. Set it and the daemon reports, at startup, any curve points that can
  have no physical effect. Measure it by sweeping duty and watching RPM.
- `min_expected_rpm` and `stall_cycles`: stall detection. The firmware's
  failsafe catches a silent *host*, but nothing else detects a dead *fan*, and a
  seized bearing or a loose 12 V lead leaves every other safeguard satisfied
  while airflow is zero. Set `min_expected_rpm` (at least 1) well below the
  fan's measured minimum speed; after `stall_cycles` consecutive cycles under it
  the daemon logs a loud error. Each reading is judged against the duty the
  controller reports applying, read before that cycle's new command, because the
  tachometer always describes the last completed second. It reports rather than
  escalating, because more duty cannot revive a dead fan and a failed tachometer
  on a working fan would just make noise. Leave it unset to disable the check;
  the daemon warns at startup if you do.
- `expected_rpm` and `stall_tolerance_percent`: the fan's measured
  `[pwm_percent, rpm]` response, which makes the stall threshold scale with the
  commanded duty. Worth setting, because an absolute floor cannot see the most
  important fault. A fan whose PWM input is held low free-runs at its *minimum*
  speed, and that speed is above any sane absolute floor, so a shorted drive
  transistor leaves the fan at its floor while the daemon commands 100%. Every
  safeguard reads healthy and airflow is a fraction of what it should be. The
  opposite fault is harmless: an *open* PWM line lets the fan's own pull-up take
  it to full speed. Measure this with the same duty sweep you used for
  `fan_floor_duty_percent`. Left unset, the daemon warns at startup that it can
  only detect a fully stopped fan.
- `poll_interval_s`: the gap between two `PWM` commands is this plus the `STATUS`
  exchange between them (bounded at 3 s, sending the command included), and that
  total must stay at or below half the firmware's failsafe timeout. With the
  default 30 s failsafe the limit is 12 s, not 15, and the daemon refuses to
  start otherwise.
- `controller_failsafe_timeout_s`: must match `CONFIG_FAN_FAILSAFE_TIMEOUT_S` in
  the firmware. It is used *only* to validate the poll interval above, and
  changes nothing on the controller. The firmware reports its real timeout, and
  the daemon checks it each time it connects: a poll interval that does not fit
  the real value stops the daemon with an error, and a value that merely differs
  gets a warning. Firmware built before it reported the timeout cannot be
  checked. With such firmware, a value set higher than the real one makes the
  daemon accept a poll interval that trips the failsafe between cycles, and the
  daemon then warns about the failsafe on each cycle that finds it. Setting it
  lower is merely conservative.
- `smoothing_alpha` and `deadband_percent`: damping. `drivetemp` reports whole
  degrees, so a drive sitting on a curve boundary dithers by 1 °C every poll,
  which on a steep curve is an audible step each cycle. The defaults (`0.3`, `2`)
  smooth the temperature input over roughly 30 s and absorb the residual jitter.
  Drive temperatures move over hours, so the lag is immaterial, and the failsafe
  paths bypass damping entirely.

Check it once by hand, then install the service. Note that `--once` is not a
passive dry run: it really commands the fan and then exits, so the controller's
failsafe takes the fan to 100% about 30 s later. The serial port is also opened
exclusively, so stop the service first if it is already running. Otherwise this
fails before touching the fan, with "Unable to acquire exclusive lock on serial
port" as root (the `flock`) or "Device or resource busy" unprivileged
(`TIOCEXCL`, which `CAP_SYS_ADMIN` bypasses).

```sh
sudo systemctl stop fan-controllerd   # if already installed
sudo fan-controllerd --once     # verbose: per-disk temps, computed duty, controller status
sudo systemctl daemon-reload && sudo systemctl enable --now fan-controllerd
```

The unit runs the daemon as a temporary unprivileged user (`DynamicUser=yes`) in
the `dialout` group, so the controller's device node must belong to `dialout`,
as it does by default on Debian. It is `Type=notify`: `systemctl start` waits
until the configuration has loaded, and fails if the daemon rejects it. Each log
line carries its own priority, so `journalctl -u fan-controllerd -p warning`
shows only the problems, such as a stalled fan.

The optional udev rule from `make install-udev` gives the controller a
`/dev/fan-controller` symlink. It matches on the board's serial, so render it
with `CONTROLLER_SERIAL` as above, or it matches nothing.

## Building a Debian package

The daemon also builds as a `.deb`. The firmware and the status page are flashed
to the microcontroller, so no package carries them.

```sh
sudo apt install build-essential debhelper cargo rustc
cd daemon && make deb
```

Like the other build steps, `make deb` runs as yourself and refuses root, and
`debian/control` declares `Rules-Requires-Root: no`, so no part of the package
build needs fakeroot either. The package is Debian-native, so its version is
the product version with no Debian revision: `1.0.0`, not `1.0.0-1`.

The daemon is Rust edition 2024, so it needs rustc 1.85 or newer. Debian trixie
ships exactly 1.85.1, which is the oldest toolchain this is built and tested
with, and `debian/control` floors the build dependency there so an older one
fails as a missing dependency rather than as a puzzling compile error.

The packaging files are in `daemon/targets/debian/`, one directory per
installation target that needs them, so a second distribution sits beside it
rather than displacing it. dpkg insists the packaging directory be literally
`./debian` at the root of the source tree, so `make deb` stages a symlink for the
duration of the build and removes it afterwards, whether or not the build
succeeded. To drive `dpkg-buildpackage` yourself, stage the same link first:

```sh
cd daemon && ln -sfn targets/debian debian && dpkg-buildpackage -us -uc -b; rm -f debian
```

`targets/debian/rules` states no path of its own: it calls `make TARGET=debian`,
which takes the layout from `config/targets/debian.conf`.

It installs `/usr/bin/fan-controllerd`, the same unit rendered for the Debian
layout, `/etc/fan-controllerd.toml` as a dpkg conffile, a `modules-load.d`
snippet that loads `drivetemp` at boot, a man page, and the udev rule as an
example under `/usr/share/doc/fan-controllerd/examples/`. README.md and
PROTOCOL.md come along when the package is built from a full checkout; they sit
outside `daemon/`, so a build from the source package alone ships without them.
The build also produces `fan-controllerd-dbgsym`, holding the debug symbols that
`dh_strip` takes out of the binary.

The unit is installed but left disabled on purpose. The shipped config names a
placeholder device and disk, and `Restart=always` would turn that into a restart
loop. Edit `/etc/fan-controllerd.toml` first, then `systemctl enable --now
fan-controllerd`.

`cargo` reaches the network for crates during the build, which an official
Debian builder forbids. To build sealed, vendor them first:

```sh
cd daemon
cargo vendor --locked vendor
mkdir -p .cargo && cat > .cargo/config.toml <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"
[source.vendored-sources]
directory = "vendor"
EOF
CARGO_OFFLINE=1 make deb
```

## Versioning

`VERSION` at the repository root is the one place the product version is written.
All three components share it, and `scripts/version` keeps them in step:

```sh
scripts/version current            # 1.0.0
scripts/version check              # fails if any component has drifted
scripts/version bump minor         # VERSION and every literal that follows it
scripts/version sync               # the literals, after editing VERSION by hand
```

The literals are the daemon's `Cargo.toml` and `Cargo.lock`, the man page's
version and date, the status page's `package.json`, and a new entry in the
Debian changelog. `Cargo.lock` is among them because it records the daemon's
own version, and a `--locked` build refuses a lockfile that disagrees with
`Cargo.toml`.

The firmware needs no syncing, because `firmware/CMakeLists.txt` reads `VERSION`
when CMake configures and passes it through as `PROJECT_VER`. That lands in the
app descriptor, so a flashed image can be identified with esptool rather than
guessed at. The daemon and the status page cannot read a file at build time, so
their literals are written by `bump` and policed by `check`, which runs on every
CI build.

The firmware's `PROTOCOL_VERSION` is deliberately left out of this. That number is
the wire contract the daemon gates on, refusing any controller whose major version
differs, so tying it to the product version would turn a cosmetic release into a
refusal to drive perfectly good firmware. It changes when the protocol changes.

## Continuous integration

`.github/workflows/build.yaml` runs on every push to a pull request and to
master, and can be dispatched by hand. Master builds are what a release takes
its artifacts from. Five jobs: a `version` job that fails the build if any
component disagrees with `VERSION`; a daemon lint job running `cargo fmt --check`
and clippy; then the daemon, the status page, whose mock tests run before it is
built, and the firmware, which waits for the page it has to embed. The three
builds each upload a release-build artifact named with the version.

The daemon builds inside a `debian:trixie` container under QEMU, so it is a real
aarch64 build of the actual `.deb` rather than a cross-compile. A second platform
is one more entry in that job's matrix. Container root only installs the build
dependencies; the build itself runs as a user with the runner's uid. The
firmware builds in the ESP-IDF image under the runner's uid as well, so no
build step runs as root anywhere.

The firmware artifact is built without `sdkconfig.secrets`, since WiFi credentials
are not CI input. It therefore has WiFi disabled: USB control works, the status
page and mDNS do not, until you rebuild it with your own credentials.

`.github/dependabot.yml` watches the daemon's Cargo lockfile, the status page's
pnpm lockfile, and the workflow files themselves, daily. The firmware is not in it:
its one dependency, `espressif/mdns`, comes from the ESP-IDF component registry,
for which Dependabot has no ecosystem. `update-firmware-components.yaml` covers
that instead, on the same daily schedule, by asking the component manager itself to
re-solve `firmware/dependencies.lock` and opening a pull request when it moves. It
re-solves within the constraint already in `firmware/main/idf_component.yml`, so
widening that for a new major version stays a manual decision.

Two pins stay manual for the same reason. The `debian:trixie` image and the
`v6.1` IDF version appear as strings inside workflow files rather than in a
Dockerfile, and nothing scans those.

`.github/workflows/release.yaml` is manual and takes a build run ID plus an
increment. The ID is that of the Build run for master's current head, which
`gh run list --workflow build.yaml --branch master --limit 1` prints. It publishes that run's artifacts unmodified, which is why it releases
the *current* `VERSION` rather than a bumped one: the version is baked into the
`.deb` and into the firmware's app descriptor, so shipping anything else would tag
a release whose contents disagree with it. Before publishing it asks the GitHub
API whether the run is a successful Build of exactly the commit being tagged,
and checks that the `.deb` declares the version being tagged. `VERSION` only
moves at release time, so the version alone cannot tell two builds of the same
cycle apart. The increment then chooses where the next cycle starts, and arrives
as a pull request rather than a push.

## Verifying end to end

1. Serial control:

   ```sh
   picocom -q --echo --omap crlf --imap lfcrlf \
       /dev/serial/by-id/usb-Espressif_USB_JTAG_serial_debug_unit_*
   ```

   Then `STATUS`, `PWM 30`, `PWM 100`: the fan audibly changes and RPM follows.

   The mappings are not optional. The firmware dispatches a line only on `\n`
   and discards `\r`, but the Return key sends `\r` and picocom's `--omap`
   defaults to empty. Without `--omap crlf` every command is swallowed and the
   board looks dead, until the accumulated bytes overflow the 63-byte line and it
   answers a single confusing `ERR line too long`. `--imap lfcrlf` keeps the
   replies from stair-stepping down the terminal, and `--echo` lets you see what
   you type, since the firmware never echoes.

2. Failsafe: stop typing for 30 s. The fan ramps to 100 %, `STATUS` shows
   `FAILSAFE=1`, and a new `PWM` clears it.
3. HTTP: watch `http://fan-controller.local/` while changing the duty. The page
   and chart track within 2 s.
4. Pull the plug: with the daemon running, unplug USB and the fan goes to 100 %
   within 30 s. Replug into *any* port and the daemon reconnects and resumes.
