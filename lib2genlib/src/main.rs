//! Convert a Liberty (.lib) file to GENLIB format for use with ABC / MapTune.
//!
//! High-performance Rust rewrite of lib2genlib.py — single-pass parser,
//! no regex, minimal allocations.

use clap::Parser as ClapParser;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(ClapParser)]
#[command(about = "Convert Liberty (.lib) to GENLIB format")]
struct Args {
    /// Input Liberty file
    input: PathBuf,
    /// Output GENLIB file (default: <input>.genlib)
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Target input transition for delay extraction (ns)
    #[arg(long, default_value_t = 0.05)]
    transition: f64,
    /// Target output load for delay extraction (pF)
    #[arg(long, default_value_t = 0.01)]
    load: f64,
    /// Print detailed info for every skipped cell
    #[arg(long)]
    debug: bool,
    /// Include cells marked dont_use
    #[arg(long)]
    include_dont_use: bool,
}

#[derive(Default, Clone)]
struct TimingArc {
    related_pin: String,
    rise_delay: f64,
    fall_delay: f64,
    timing_sense: String,
}

#[derive(Default, Clone)]
struct Pin {
    name: String,
    direction: String,
    function: String,
    capacitance: f64,
    max_capacitance: f64,
    three_state: String,
    timing_arcs: Vec<TimingArc>,
}

#[derive(Default, Clone)]
struct Cell {
    name: String,
    area: f64,
    pins: Vec<Pin>,
    is_sequential: bool,
    dont_use: bool,
    dont_touch: bool,
}

// ---------------------------------------------------------------------------
// Fast Liberty parser — no regex, single-pass
// ---------------------------------------------------------------------------

fn strip_comments(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if i + 1 < input.len() && input[i] == b'/' && input[i + 1] == b'*' {
            i += 2;
            while i + 1 < input.len() && !(input[i] == b'*' && input[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else if i + 1 < input.len() && input[i] == b'/' && input[i + 1] == b'/' {
            while i < input.len() && input[i] != b'\n' {
                i += 1;
            }
        } else {
            out.push(input[i]);
            i += 1;
        }
    }
    out
}

#[inline]
fn skip_ws(data: &[u8], mut pos: usize) -> usize {
    while pos < data.len() && data[pos].is_ascii_whitespace() {
        pos += 1;
    }
    pos
}

fn read_attr_value(data: &[u8], pos: usize) -> (String, usize) {
    let pos = skip_ws(data, pos);
    let start = pos;
    let mut end = pos;
    while end < data.len() && data[end] != b';' && data[end] != b'{' && data[end] != b'}' {
        end += 1;
    }
    let val = String::from_utf8_lossy(&data[start..end])
        .trim()
        .trim_matches('"')
        .to_string();
    let next = if end < data.len() && data[end] == b';' { end + 1 } else { end };
    (val, next)
}

fn extract_group_body(data: &[u8], pos: usize) -> (&[u8], usize) {
    let start = pos;
    let mut depth = 1;
    let mut i = pos;
    while i < data.len() && depth > 0 {
        match data[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    (&data[start..i.saturating_sub(1)], i)
}

fn parse_nldm_table(body: &[u8]) -> Vec<Vec<f64>> {
    let needle = b"values";
    let Some(vpos) = body.windows(needle.len()).position(|w| w.eq_ignore_ascii_case(needle)) else {
        return vec![];
    };
    let mut pos = vpos + needle.len();
    pos = skip_ws(body, pos);
    if pos >= body.len() || body[pos] != b'(' { return vec![]; }
    pos += 1;
    let mut depth = 1;
    let start = pos;
    while pos < body.len() && depth > 0 {
        match body[pos] { b'(' => depth += 1, b')' => depth -= 1, _ => {} }
        pos += 1;
    }
    let inner = &body[start..pos.saturating_sub(1)];
    let raw = String::from_utf8_lossy(inner);
    let mut rows = Vec::new();
    for segment in raw.split('"') {
        let segment = segment.trim().trim_matches(',').trim();
        if segment.is_empty() || segment == "\\" { continue; }
        let vals: Vec<f64> = segment.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if !vals.is_empty() { rows.push(vals); }
    }
    rows
}

fn parse_index(body: &[u8], name: &str) -> Vec<f64> {
    let needle = name.as_bytes();
    let Some(pos) = body.windows(needle.len()).position(|w| w == needle) else { return vec![]; };
    let mut p = pos + needle.len();
    p = skip_ws(body, p);
    if p >= body.len() || body[p] != b'(' { return vec![]; }
    p += 1;
    let start = memchr::memchr(b'"', &body[p..]).map(|i| p + i + 1);
    let Some(start) = start else { return vec![]; };
    let end = memchr::memchr(b'"', &body[start..]).map(|i| start + i);
    let Some(end) = end else { return vec![]; };
    String::from_utf8_lossy(&body[start..end])
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect()
}

fn closest_idx(vals: &[f64], target: f64) -> usize {
    vals.iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| ((**a - target).abs()).partial_cmp(&(**b - target).abs()).unwrap())
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn pick_delay_from_table(body: &[u8], idx1_target: f64, idx2_target: f64) -> f64 {
    let idx1 = parse_index(body, "index_1");
    let idx2 = parse_index(body, "index_2");
    let rows = parse_nldm_table(body);
    if rows.is_empty() { return 0.0; }
    if idx1.is_empty() || idx2.is_empty() {
        let mr = rows.len() / 2;
        return rows[mr][rows[mr].len() / 2];
    }
    let r = closest_idx(&idx1, idx1_target);
    let c = closest_idx(&idx2, idx2_target);
    if r < rows.len() && c < rows[r].len() { rows[r][c] }
    else { rows[rows.len() / 2][rows[0].len() / 2] }
}

fn for_each_group<'a>(data: &'a [u8], group_type: &str, mut cb: impl FnMut(&str, &'a [u8])) {
    let gt = group_type.as_bytes();
    let mut pos = 0;
    while pos + gt.len() < data.len() {
        let found = data[pos..].windows(gt.len()).position(|w| w.eq_ignore_ascii_case(gt));
        let Some(offset) = found else { break };
        let match_pos = pos + offset;
        if match_pos > 0 && (data[match_pos - 1].is_ascii_alphanumeric() || data[match_pos - 1] == b'_') {
            pos = match_pos + gt.len();
            continue;
        }
        let after = skip_ws(data, match_pos + gt.len());
        if after >= data.len() || data[after] != b'(' { pos = after; continue; }
        let name_start = after + 1;
        let mut p = name_start;
        while p < data.len() && data[p] != b')' { p += 1; }
        let name = String::from_utf8_lossy(&data[name_start..p]).trim().trim_matches('"').to_string();
        p += 1;
        p = skip_ws(data, p);
        if p >= data.len() || data[p] != b'{' { pos = p; continue; }
        p += 1;
        let (body, end) = extract_group_body(data, p);
        cb(&name, body);
        pos = end;
    }
}

fn get_attr(data: &[u8], attr: &str) -> String {
    let needle = attr.as_bytes();
    let mut pos = 0;
    while pos + needle.len() < data.len() {
        let found = data[pos..].windows(needle.len()).position(|w| w.eq_ignore_ascii_case(needle));
        let Some(offset) = found else { break };
        let match_pos = pos + offset;
        if match_pos > 0 && (data[match_pos - 1].is_ascii_alphanumeric() || data[match_pos - 1] == b'_') {
            pos = match_pos + needle.len();
            continue;
        }
        let after_word = match_pos + needle.len();
        if after_word < data.len() && (data[after_word].is_ascii_alphanumeric() || data[after_word] == b'_') {
            pos = after_word;
            continue;
        }
        let mut p = skip_ws(data, after_word);
        if p >= data.len() || data[p] != b':' { pos = p; continue; }
        p += 1;
        let (val, _) = read_attr_value(data, p);
        return val;
    }
    String::new()
}

fn get_attr_f64(data: &[u8], attr: &str) -> f64 {
    get_attr(data, attr).parse().unwrap_or(0.0)
}

fn get_attr_bool(data: &[u8], attr: &str) -> bool {
    get_attr(data, attr).eq_ignore_ascii_case("true")
}

fn has_group(data: &[u8], group_type: &str) -> bool {
    let gt = group_type.as_bytes();
    let mut pos = 0;
    while pos + gt.len() < data.len() {
        let found = data[pos..].windows(gt.len()).position(|w| w.eq_ignore_ascii_case(gt));
        let Some(offset) = found else { break };
        let match_pos = pos + offset;
        if match_pos > 0 && (data[match_pos - 1].is_ascii_alphanumeric() || data[match_pos - 1] == b'_') {
            pos = match_pos + gt.len();
            continue;
        }
        let after = skip_ws(data, match_pos + gt.len());
        if after < data.len() && data[after] == b'(' { return true; }
        pos = after;
    }
    false
}

fn extract_delay_from_timing(timing_body: &[u8], target_transition: f64, target_load: f64) -> (f64, f64) {
    let mut rise = 0.0_f64;
    let mut fall = 0.0_f64;
    for_each_group(timing_body, "cell_rise", |_, tbl| {
        rise = rise.max(pick_delay_from_table(tbl, target_transition, target_load));
    });
    for_each_group(timing_body, "cell_fall", |_, tbl| {
        fall = fall.max(pick_delay_from_table(tbl, target_transition, target_load));
    });
    if rise == 0.0 && fall == 0.0 {
        for_each_group(timing_body, "rise_transition", |_, tbl| {
            rise = rise.max(pick_delay_from_table(tbl, target_transition, target_load));
        });
        for_each_group(timing_body, "fall_transition", |_, tbl| {
            fall = fall.max(pick_delay_from_table(tbl, target_transition, target_load));
        });
    }
    (rise, fall)
}

fn parse_liberty(data: &[u8], target_transition: f64, target_load: f64) -> Vec<Cell> {
    let data = strip_comments(data);
    let mut cells = Vec::new();

    for_each_group(&data, "cell", |cell_name, cell_body| {
        let mut c = Cell {
            name: cell_name.to_string(),
            area: get_attr_f64(cell_body, "area"),
            dont_use: get_attr_bool(cell_body, "dont_use"),
            dont_touch: get_attr_bool(cell_body, "dont_touch"),
            ..Default::default()
        };

        for seq in &["ff", "ff_bank", "latch", "latch_bank", "statetable"] {
            if has_group(cell_body, seq) { c.is_sequential = true; break; }
        }
        if !c.is_sequential && !get_attr(cell_body, "clock_gating_integrated_cell").is_empty() {
            c.is_sequential = true;
        }

        let parse_pin = |pin_name: &str, pin_body: &[u8]| -> Pin {
            let mut p = Pin {
                name: pin_name.to_string(),
                direction: get_attr(pin_body, "direction").to_ascii_lowercase(),
                function: get_attr(pin_body, "function"),
                capacitance: get_attr_f64(pin_body, "capacitance"),
                max_capacitance: get_attr_f64(pin_body, "max_capacitance"),
                three_state: get_attr(pin_body, "three_state"),
                ..Default::default()
            };
            if p.capacitance == 0.0 {
                let rc = get_attr_f64(pin_body, "rise_capacitance");
                let fc = get_attr_f64(pin_body, "fall_capacitance");
                if rc > 0.0 || fc > 0.0 { p.capacitance = rc.max(fc); }
            }
            for_each_group(pin_body, "timing", |_, timing_body| {
                let (rd, fd) = extract_delay_from_timing(timing_body, target_transition, target_load);
                p.timing_arcs.push(TimingArc {
                    related_pin: get_attr(timing_body, "related_pin"),
                    timing_sense: get_attr(timing_body, "timing_sense"),
                    rise_delay: rd,
                    fall_delay: fd,
                });
            });
            p
        };

        for_each_group(cell_body, "pin", |name, body| { c.pins.push(parse_pin(name, body)); });
        for_each_group(cell_body, "bundle", |_, bundle_body| {
            let bdir = get_attr(bundle_body, "direction").to_ascii_lowercase();
            let bcap = get_attr_f64(bundle_body, "capacitance");
            let bfunc = get_attr(bundle_body, "function");
            for_each_group(bundle_body, "pin", |name, body| {
                let mut p = parse_pin(name, body);
                if p.direction.is_empty() { p.direction = bdir.clone(); }
                if p.capacitance == 0.0 { p.capacitance = bcap; }
                if p.function.is_empty() { p.function = bfunc.clone(); }
                c.pins.push(p);
            });
        });
        for_each_group(cell_body, "bus", |_, bus_body| {
            let bdir = get_attr(bus_body, "direction").to_ascii_lowercase();
            for_each_group(bus_body, "pin", |name, body| {
                let mut p = parse_pin(name, body);
                if p.direction.is_empty() { p.direction = bdir.clone(); }
                c.pins.push(p);
            });
        });

        cells.push(c);
    });
    cells
}

// ---------------------------------------------------------------------------
// GENLIB conversion
// ---------------------------------------------------------------------------

fn infer_phase_from_sense(sense: &str) -> &'static str {
    match sense {
        "positive_unate" => "NONINV",
        "negative_unate" => "INV",
        _ => "UNKNOWN",
    }
}

fn infer_phase_from_function(function: &str, input_pin: &str) -> &'static str {
    let func = function.replace(' ', "");
    let has_inv = func.contains(&format!("!{input_pin}"))
        || func.contains(&format!("~{input_pin}"))
        || func.contains(&format!("{input_pin}'"));
    let has_noninv = func.match_indices(input_pin).any(|(i, _)| {
        let before_ok = i == 0
            || (!func.as_bytes()[i - 1].is_ascii_alphanumeric()
                && func.as_bytes()[i - 1] != b'!'
                && func.as_bytes()[i - 1] != b'~');
        let after = i + input_pin.len();
        let after_ok = after >= func.len()
            || (!func.as_bytes()[after].is_ascii_alphanumeric() && func.as_bytes()[after] != b'\'');
        before_ok && after_ok
    });
    if has_inv && !has_noninv { "INV" }
    else if has_noninv && !has_inv { "NONINV" }
    else { "UNKNOWN" }
}

fn normalize_function(func: &str) -> String {
    let mut out = String::with_capacity(func.len());
    for b in func.bytes() {
        match b {
            b'~' => out.push('!'),
            b'&' => out.push('*'),
            b'|' => out.push('+'),
            b'\'' => {
                let end = out.len();
                let mut start = end;
                while start > 0 && (out.as_bytes()[start - 1].is_ascii_alphanumeric() || out.as_bytes()[start - 1] == b'_') {
                    start -= 1;
                }
                if start < end {
                    let ident: String = out[start..end].to_string();
                    out.truncate(start);
                    out.push('!');
                    out.push_str(&ident);
                }
            }
            _ => out.push(b as char),
        }
    }
    out = out.replace("1'b0", "CONST0").replace("1'b1", "CONST1");
    out
}

fn cell_to_genlib(cell: &Cell) -> Result<String, &'static str> {
    if cell.dont_use { return Err("dont_use"); }
    if cell.dont_touch { return Err("dont_touch"); }
    if cell.is_sequential { return Err("sequential"); }

    let output_pins: Vec<&Pin> = cell.pins.iter()
        .filter(|p| p.direction == "output" && !p.function.is_empty() && p.three_state.is_empty())
        .collect();
    let input_pins: Vec<&Pin> = cell.pins.iter().filter(|p| p.direction == "input").collect();
    let tristate_only = cell.pins.iter().any(|p| p.direction == "output" && !p.three_state.is_empty())
        && output_pins.is_empty();

    if tristate_only { return Err("tristate_only"); }
    if output_pins.is_empty() {
        if cell.pins.iter().any(|p| p.direction == "output") { return Err("output_no_function"); }
        if cell.pins.iter().any(|p| p.direction == "inout") { return Err("inout_only"); }
        return Err("no_output_pin");
    }
    if input_pins.is_empty() {
        let has_const = output_pins.iter().any(|p| matches!(p.function.trim(), "0" | "1" | "1'b0" | "1'b1"));
        if !has_const { return Err("no_input_pin"); }
    }
    if output_pins.len() > 1 { return Err("multi_output"); }

    let out = output_pins[0];
    let func = normalize_function(&out.function);

    let pin_set: HashMap<&str, ()> = cell.pins.iter().map(|p| (p.name.as_str(), ())).collect();
    let mut pin_delays: HashMap<String, (f64, f64, String)> = HashMap::new();
    for arc in &out.timing_arcs {
        if !arc.related_pin.is_empty() && pin_set.contains_key(arc.related_pin.as_str()) {
            let e = pin_delays.entry(arc.related_pin.clone()).or_insert((0.0, 0.0, String::new()));
            e.0 = e.0.max(arc.rise_delay);
            e.1 = e.1.max(arc.fall_delay);
            if e.2.is_empty() && !arc.timing_sense.is_empty() { e.2 = arc.timing_sense.clone(); }
        }
    }

    let worst_rise = out.timing_arcs.iter().map(|a| a.rise_delay).fold(0.0_f64, f64::max);
    let worst_fall = out.timing_arcs.iter().map(|a| a.fall_delay).fold(0.0_f64, f64::max);

    let mut result = format!("GATE {:<30} {:>10.4} {}={}", cell.name, cell.area, out.name, func);
    for ip in &input_pins {
        let (rise, fall, sense) = pin_delays.get(&ip.name).cloned().unwrap_or((worst_rise, worst_fall, String::new()));
        let phase = if !sense.is_empty() { infer_phase_from_sense(&sense) } else { infer_phase_from_function(&out.function, &ip.name) };
        let load = if ip.capacitance > 0.0 { ip.capacitance } else { 0.001 };
        let max_load = if ip.max_capacitance > 0.0 { ip.max_capacitance } else { 999.0 };
        result.push_str(&format!("\n  PIN {:<10} {:<8} {:.4} {:.4} {:.4} {:.4}", ip.name, phase, load, max_load, rise, fall));
    }
    Ok(result)
}

fn main() {
    let args = Args::parse();
    let output = args.output.unwrap_or_else(|| args.input.with_extension("genlib"));
    let data = fs::read(&args.input).unwrap_or_else(|e| {
        eprintln!("Error reading {}: {}", args.input.display(), e);
        std::process::exit(1);
    });

    let cells = parse_liberty(&data, args.transition, args.load);
    let mut lines: Vec<String> = vec![
        format!("# GENLIB converted from {}", args.input.file_name().unwrap().to_string_lossy()),
        format!("# Transition={}, Load={}", args.transition, args.load),
        String::new(),
    ];

    let mut has_const0 = false;
    let mut has_const1 = false;
    let mut converted = 0u32;
    let mut skip_counts: HashMap<&str, u32> = HashMap::new();
    let mut skip_examples: HashMap<&str, Vec<String>> = HashMap::new();

    for cell in &cells {
        let mut cell_copy;
        let cell_ref = if args.include_dont_use && cell.dont_use {
            cell_copy = cell.clone();
            cell_copy.dont_use = false;
            &cell_copy
        } else { cell };

        match cell_to_genlib(cell_ref) {
            Ok(genlib) => {
                for p in &cell_ref.pins {
                    if p.direction == "output" {
                        match p.function.trim() {
                            "0" | "1'b0" => has_const0 = true,
                            "1" | "1'b1" => has_const1 = true,
                            _ => {}
                        }
                    }
                }
                lines.push(genlib);
                lines.push(String::new());
                converted += 1;
            }
            Err(reason) => {
                *skip_counts.entry(reason).or_insert(0) += 1;
                let examples = skip_examples.entry(reason).or_default();
                if examples.len() < 5 { examples.push(cell.name.clone()); }
            }
        }
    }

    if !has_const0 { lines.insert(3, "GATE _const0_             0.0000 Z=CONST0;\n".to_string()); }
    if !has_const1 { lines.insert(3, "GATE _const1_             0.0000 Z=CONST1;\n".to_string()); }

    let mut out_file = fs::File::create(&output).unwrap_or_else(|e| {
        eprintln!("Error creating {}: {}", output.display(), e);
        std::process::exit(1);
    });
    write!(out_file, "{}", lines.join("\n")).unwrap();

    let total = cells.len();
    eprintln!("Total cells in Liberty: {total}");
    eprintln!("Converted:              {converted}");
    eprintln!("Skipped:                {}", total as u32 - converted);
    eprintln!();
    if !skip_counts.is_empty() {
        eprintln!("Skip breakdown:");
        let mut sorted: Vec<_> = skip_counts.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        for (reason, count) in sorted {
            let examples = skip_examples.get(reason).map(|v| v.join(", ")).unwrap_or_default();
            eprintln!("  {:<25} {:>5}  e.g. {}", reason, count, examples);
        }
    }
    if args.debug {
        eprintln!("\n--- Debug: all skipped cells ---");
        for cell in &cells {
            let mut cell_copy;
            let cell_ref = if args.include_dont_use && cell.dont_use {
                cell_copy = cell.clone(); cell_copy.dont_use = false; &cell_copy
            } else { cell };
            if let Err(reason) = cell_to_genlib(cell_ref) {
                eprintln!("\n{} -> {}", cell.name, reason);
                eprintln!("  area={} seq={} dont_use={} dont_touch={}", cell.area, cell.is_sequential, cell.dont_use, cell.dont_touch);
                for p in &cell.pins {
                    eprintln!("    {}: dir={} func='{}' tristate='{}' cap={}", p.name, p.direction, p.function, p.three_state, p.capacitance);
                }
            }
        }
    }
    eprintln!("\nWritten to: {}", output.display());
}
