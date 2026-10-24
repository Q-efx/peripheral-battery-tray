"""Generate src/razer_devices.rs from an OpenRazer checkout.

Usage: python3 gen_razer.py <openrazer_dir> <out.rs>
"""
import re
import sys

root, out = sys.argv[1], sys.argv[2]
drv = open(f"{root}/driver/razermouse_driver.c").read()
hdr = open(f"{root}/driver/razermouse_driver.h").read()
daemon = open(f"{root}/daemon/openrazer_daemon/hardware/mouse.py").read()

defines = {m.group(1): int(m.group(2), 16)
           for m in re.finditer(r"#define\s+USB_DEVICE_ID_RAZER_(\w+)\s+(0x[0-9A-Fa-f]+)", hdr)}


def func_body(name):
    i = drv.index(name + "(")
    i = drv.index("{", i)
    depth, j = 0, i
    while True:
        if drv[j] == "{":
            depth += 1
        elif drv[j] == "}":
            depth -= 1
            if depth == 0:
                return drv[i:j]
        j += 1


def case_groups(body):
    """Yield (list_of_case_names, following_code) for each case group in a switch."""
    groups, cur = [], []
    for line in body.splitlines():
        m = re.match(r"\s*case USB_DEVICE_ID_RAZER_(\w+):", line)
        if m:
            cur.append(m.group(1))
            continue
        if cur and line.strip():
            groups.append((cur, line.strip()))
            cur = []
    return groups


# Transaction id per device (battery request)
tid = {}
for names, code in case_groups(func_body("razer_attr_read_charge_level")):
    m = re.search(r"transaction_id\.id = (0x[0-9A-Fa-f]+)", code)
    if m:
        for n in names:
            tid[n] = int(m.group(1), 16)

# Devices for which the driver actually exposes the charge_level / charge_status
# files at probe time (some PIDs appear in the read switch but never get the file).
def probe_attrs():
    body = func_body("razer_mouse_probe")
    active, level, status = set(), set(), set()
    for line in body.splitlines():
        m = re.match(r"\s*case USB_DEVICE_ID_RAZER_(\w+):", line)
        if m:
            # Cases accumulate until a break: earlier groups fall through into later code.
            active.add(m.group(1))
            continue
        s = line.strip()
        if s.startswith("break;") or s.startswith("default:") or s.startswith("switch"):
            active = set()
            continue
        if "&dev_attr_charge_level)" in s:
            level |= active
        if "&dev_attr_charge_status)" in s:
            status |= active
    return level, status


has_level, has_status = probe_attrs()

# Devices whose charge_status read hard-codes "0" (AA batteries)
no_charge = set()
for names, code in case_groups(func_body("razer_attr_read_charge_status")):
    if 'sysfs_emit(buf, "0\\n")' in code:
        no_charge.update(names)
# Devices that send a real charging request
charge_req = set()
for names, code in case_groups(func_body("razer_attr_read_charge_status")):
    if "transaction_id.id" in code:
        charge_req.update(names)

dropped = sorted(d for d in tid if d not in has_level)
if dropped:
    print("no charge_level file at probe, skipped:", ", ".join(dropped))

# Wait time + report index per device
waits = {"RAZER_MOUSE_WAIT_US": 600, "RAZER_NEW_MOUSE_RECEIVER_WAIT_US": 31000,
         "RAZER_ATHERIS_RECEIVER_WAIT_US": 400000, "RAZER_VIPER_MOUSE_RECEIVER_WAIT_US": 59900}
for k in waits:
    m = re.search(rf"#define {k} (\d+)", hdr)
    waits[k] = int(m.group(1))
wait, index = {}, {}
body = func_body("razer_get_report")
pending = []
for line in body.splitlines():
    m = re.match(r"\s*case USB_DEVICE_ID_RAZER_(\w+):", line)
    if m:
        pending.append(m.group(1))
        continue
    if "index = 0x03" in line:
        for n in pending:
            index[n] = 3
    m = re.search(r"(RAZER_\w+_WAIT_US)", line)
    if m and pending:
        for n in pending:
            wait[n] = waits[m.group(1)]
        pending = []
    if re.match(r"\s*default:", line):
        pending = []

# Product names from the daemon: "Class for the Razer X" ... USB_PID = 0x....
names_by_pid = {}
for m in re.finditer(r'"""\s*\n\s*Class for the (Razer[^\n]*?)\s*\n.*?USB_PID = (0x[0-9A-Fa-f]+)', daemon, re.S):
    name, pid = m.group(1).strip(), int(m.group(2), 16)
    if "\"\"\"" in name:
        continue
    names_by_pid.setdefault(pid, name)


def pretty(define):
    return "Razer " + define.replace("_", " ").title()


rows = []
for d, t in sorted(tid.items(), key=lambda kv: defines.get(kv[0], 0)):
    if d not in defines or d not in has_level:
        continue
    pid = defines[d]
    can_charge = d in has_status and d in charge_req and d not in no_charge
    rows.append((pid, names_by_pid.get(pid, pretty(d)), t,
                 wait.get(d, waits["RAZER_MOUSE_WAIT_US"]), index.get(d, 0), can_charge))

seen = set()
lines = []
for pid, name, t, w, idx, chg in rows:
    if pid in seen:
        continue
    seen.add(pid)
    lines.append(f"    RazerModel {{ pid: 0x{pid:04X}, name: {name!r}, transaction_id: 0x{t:02X}, "
                 f"wait_us: {w}, interface: {idx}, has_charging: {str(chg).lower()} }},".replace("'", '"'))

with open(out, "w") as f:
    f.write("// @generated by tools/gen_razer.py from OpenRazer's mouse driver (GPL-2.0).\n")
    f.write("// Device facts only: USB product ids, transaction ids and response delays.\n\n")
    f.write("use crate::razer::RazerModel;\n\n")
    f.write("pub const RAZER_MODELS: &[RazerModel] = &[\n")
    f.write("\n".join(lines))
    f.write("\n];\n")
print(f"{len(lines)} battery-capable Razer models written")
