.pragma library

// Pure helpers for the TUXEDO stats widget. No I/O: the panel feeds them the
// JSON printed by `tcc-cli --json stats` and `tcc-cli --json profile list`.

function parseJson(text) {
  try {
    var value = JSON.parse(String(text))
    return value && typeof value === "object" ? value : null
  } catch (e) {
    return null
  }
}

// "12 %" style number, or "—" when the value is missing.
function number(value, decimals, unit) {
  if (typeof value !== "number" || !isFinite(value)) return "—"
  return value.toFixed(decimals) + (unit ? " " + unit : "")
}

// Short label for the bar: CPU load and battery power.
function barLabel(stats) {
  if (!stats || !stats.daemon) return ""
  var parts = []
  if (stats.cpu && typeof stats.cpu.usagePercent === "number")
    parts.push(Math.round(stats.cpu.usagePercent) + "%")
  if (stats.battery && stats.battery.present && typeof stats.battery.powerW === "number"
      && stats.battery.powerW > 0.05)
    parts.push(stats.battery.powerW.toFixed(1) + "W")
  return parts.join(" ")
}

// Profiles as [{id, name, active}], active first match wins; empty on bad data.
function profiles(list) {
  if (!Array.isArray(list)) return []
  var out = []
  for (var i = 0; i < list.length && i < 32; i++) {
    var p = list[i]
    if (p && typeof p.id === "string" && typeof p.name === "string")
      out.push({ id: p.id, name: p.name, active: p.active === true })
  }
  return out
}
