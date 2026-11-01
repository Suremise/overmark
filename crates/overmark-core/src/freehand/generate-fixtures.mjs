// Writes fixtures.rs beside this file: outlines made by perfect-freehand itself, which the port's
// tests compare against. Build-only; nothing here ships.
//
//   npm pack perfect-freehand@1.2.3        (in a folder of its own, then unpack it)
//   node generate-fixtures.mjs <that folder>/package/dist/cjs/index.js
//
// Each case is an input and options. Easings are named here and in tests.rs, as functions
// can't be written to a file.

import { createRequire } from 'node:module'
import { writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const library = process.argv[2]
if (!library) {
  console.error('usage: node generate-fixtures.mjs <perfect-freehand dist/cjs/index.js>')
  process.exit(2)
}
const pf = createRequire(import.meta.url)(library)
const VERSION = '1.2.3'

const EASINGS = {
  linear: (t) => t,
  quad_in: (t) => t * t,
  sine_out: (t) => Math.sin((t * Math.PI) / 2),
}

// Inputs: [x, y] or [x, y, pressure], made from formulas so they're the same every time.
const range = (n, f) => Array.from({ length: n }, (_, i) => f(i))
let seed = 12345
const random = () => {
  seed = (seed * 1103515245 + 12345) % 2147483648
  return seed / 2147483648
}
const INPUTS = {
  line: range(20, (i) => [i * 10, 0]),
  wave: range(40, (i) => [i * 6, Math.sin(i / 4) * 40]),
  spiral_pressure: range(40, (i) => {
    const a = i * 0.3
    const r = 10 + i * 2
    return [100 + Math.cos(a) * r, 100 + Math.sin(a) * r, 0.2 + 0.6 * Math.abs(Math.sin(i / 6))]
  }),
  zigzag: [0, 1, 2, 3, 4].flatMap((k) =>
    range(8, (i) => [k * 40 + i * 5, (k % 2 === 0 ? i : 8 - i) * 5])
  ),
  u_turn: [...range(15, (i) => [i * 8, 0]), ...range(15, (i) => [112 - i * 8, 0.5])],
  jitter: range(30, (i) => [i * 4 + random() * 3, 50 + random() * 6, random()]),
  dot: [[50, 60]],
  dot_pressure: [[50, 60, 0.9]],
  two_points: [
    [10, 10],
    [60, 40],
  ],
  repeats: [[0, 0], [0, 0], [5, 5], [5, 5], [5, 5], [30, 10], [60, 20], [60, 20], [90, 25]],
  bad_pressure: [[0, 0, -1], [20, 5, NaN], [40, 10, 0.3], [60, 20], [80, 25, 1]],
}

// Options. `easing`, `start.easing` and `end.easing` name an entry of EASINGS.
const OPTIONS = {
  defaults: {},
  fine: { size: 8, thinning: 0.7, smoothing: 0.3, streamline: 0.3 },
  no_thinning: { thinning: 0 },
  real_pressure: { simulatePressure: false },
  tapers: { start: { taper: 30 }, end: { taper: true } },
  flat_caps: { start: { cap: false }, end: { cap: false } },
  last: { last: true },
  no_streamline: { streamline: 0 },
  full_streamline: { streamline: 1 },
  no_smoothing: { smoothing: 0 },
  negative_thinning: { thinning: -0.5 },
  eased: {
    easing: 'quad_in',
    start: { taper: 20, easing: 'linear' },
    end: { taper: 20, easing: 'sine_out' },
  },
  tapered_last: { last: true, start: { taper: true }, end: { taper: 15 } },
  zero_size: { size: 0 },
}

// Which options each input is drawn with.
const ALL = Object.keys(OPTIONS)
const SOME = ['defaults', 'tapers', 'last', 'real_pressure', 'flat_caps', 'tapered_last']
const CASES = [
  ...['wave', 'spiral_pressure', 'zigzag'].flatMap((input) => ALL.map((o) => [input, o])),
  ...['line', 'u_turn', 'jitter', 'dot', 'dot_pressure', 'two_points', 'repeats', 'bad_pressure'].flatMap(
    (input) => SOME.map((o) => [input, o])
  ),
]

function resolve(options) {
  const out = { ...options }
  if (out.easing) out.easing = EASINGS[out.easing]
  for (const end of ['start', 'end']) {
    if (out[end]) {
      out[end] = { ...out[end] }
      if (out[end].easing) out[end].easing = EASINGS[out[end].easing]
    }
  }
  return out
}

// A number as a Rust f64 literal: JavaScript's shortest round-trip form, which Rust reads back
// exactly.
function num(n) {
  if (Number.isNaN(n)) return 'f64::NAN'
  if (n === Infinity) return 'f64::INFINITY'
  if (n === -Infinity) return 'f64::NEG_INFINITY'
  const s = String(n)
  return /[.e]/.test(s) ? s : `${s}.0`
}

function rustInput(points) {
  return points
    .map(([x, y, p]) => `(${num(x)}, ${num(y)}, ${p === undefined ? 'None' : `Some(${num(p)})`})`)
    .join(', ')
}

function rustTaper(taper) {
  if (taper === undefined || taper === false) return 'Taper::None'
  if (taper === true) return 'Taper::Full'
  return `Taper::Distance(${num(taper)})`
}

function rustEnd(end, fallback) {
  if (!end) return fallback
  const parts = [`..${fallback}`]
  if (end.cap !== undefined) parts.unshift(`cap: ${end.cap}`)
  if (end.taper !== undefined) parts.unshift(`taper: ${rustTaper(end.taper)}`)
  if (end.easing) parts.unshift(`easing: ${end.easing}`)
  return `StrokeEnd { ${parts.join(', ')} }`
}

function rustOptions(o) {
  const fields = []
  if (o.size !== undefined) fields.push(`size: ${num(o.size)}`)
  if (o.thinning !== undefined) fields.push(`thinning: ${num(o.thinning)}`)
  if (o.smoothing !== undefined) fields.push(`smoothing: ${num(o.smoothing)}`)
  if (o.streamline !== undefined) fields.push(`streamline: ${num(o.streamline)}`)
  if (o.easing) fields.push(`easing: ${o.easing}`)
  if (o.simulatePressure !== undefined) fields.push(`simulate_pressure: ${o.simulatePressure}`)
  if (o.start) fields.push(`start: ${rustEnd(o.start, 'StrokeEnd::START')}`)
  if (o.end) fields.push(`end: ${rustEnd(o.end, 'StrokeEnd::END')}`)
  if (o.last !== undefined) fields.push(`last: ${o.last}`)
  fields.push('..defaults()')
  return `StrokeOptions { ${fields.join(', ')} }`
}

let out = `// Generated by generate-fixtures.mjs from perfect-freehand ${VERSION}: don't edit by hand.
// Each case: an input, the options, and the outline the library returned (x, y pairs).

use super::*;
use super::tests::{defaults, linear, quad_in, sine_out};

pub struct Case {
    pub name: &'static str,
    pub input: &'static [(f64, f64, Option<f64>)],
    pub options: fn() -> StrokeOptions,
    pub outline: &'static [f64],
}

pub static CASES: &[Case] = &[
`
let count = 0
for (const [inputName, optionsName] of CASES) {
  const input = INPUTS[inputName]
  const outline = pf.getStroke(input, resolve(OPTIONS[optionsName]))
  out += `    Case {
        name: "${inputName}/${optionsName}",
        input: &[${rustInput(input)}],
        options: || ${rustOptions(OPTIONS[optionsName])},
        outline: &[${outline.map(([x, y]) => `${num(x)}, ${num(y)}`).join(', ')}],
    },
`
  count++
}
out += '];\n'

// The measured points for a few cases, to check getStrokePoints on its own.
out += `
pub struct PointsCase {
    pub name: &'static str,
    pub input: &'static [(f64, f64, Option<f64>)],
    pub options: fn() -> StrokeOptions,
    /// x, y, pressure, distance, vector x, vector y, running length, per point.
    pub points: &'static [f64],
}

pub static POINTS_CASES: &[PointsCase] = &[
`
for (const [inputName, optionsName] of [
  ['wave', 'defaults'],
  ['spiral_pressure', 'last'],
  ['two_points', 'defaults'],
  ['dot_pressure', 'defaults'],
  ['repeats', 'no_streamline'],
  ['bad_pressure', 'defaults'],
]) {
  const input = INPUTS[inputName]
  const points = pf.getStrokePoints(input, resolve(OPTIONS[optionsName]))
  const flat = points.flatMap((p) => [
    p.point[0],
    p.point[1],
    p.pressure,
    p.distance,
    p.vector[0],
    p.vector[1],
    p.runningLength,
  ])
  out += `    PointsCase {
        name: "${inputName}/${optionsName}",
        input: &[${rustInput(input)}],
        options: || ${rustOptions(OPTIONS[optionsName])},
        points: &[${flat.map(num).join(', ')}],
    },
`
}
out += '];\n'

const target = join(dirname(fileURLToPath(import.meta.url)), 'fixtures.rs')
writeFileSync(target, out)
console.log(`wrote ${count} outline cases to ${target}`)
