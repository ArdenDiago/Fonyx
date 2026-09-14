.pragma library
// dB<->gain helpers, ported verbatim from ui/index.html's own constants —
// METER_DB_MIN/MAX define both the fader range and the meter/tick mapping
// everywhere in this app, same as there.
var METER_DB_MIN = -40;
var METER_DB_MAX = 12;

function ampToDb(amp) { return amp > 1e-4 ? 20 * Math.log10(amp) : METER_DB_MIN; }
function gainToDb(gain) { return gain > 1e-4 ? 20 * Math.log10(gain) : METER_DB_MIN; }
function dbToGain(db) { db = parseFloat(db); return db <= METER_DB_MIN ? 0 : Math.pow(10, db / 20); }
function dbToFrac(db) { return Math.max(0, Math.min(1, (db - METER_DB_MIN) / (METER_DB_MAX - METER_DB_MIN))); }
function gainToDbText(gain) {
    if (gain <= 1e-4) return "-∞ dB";
    var db = gainToDb(gain);
    return (db >= 0 ? "+" : "") + db.toFixed(1) + " dB";
}
