//! Per-period measures, and the one property that makes them trustworthy: the streaming
//! accumulator and `gs-waveform-measurements`' batch functions must agree on the same data.
//!
//! Without that, a `kind=measure` RMS and a per-period RMS over the same window would differ in
//! their last digits by a quadrature rule nobody chose, and no reader could tell which to
//! believe.

use std::collections::BTreeMap;

use gs_waveform_measurements::{Sample as WaveSample, Window};
use gsim_rules::{
    Action, Condition, EventRecord, PeriodBoundary, PeriodMeasure, Rule, RuleEngine, Sample,
    Severity, Watch,
};

struct Step {
    values: BTreeMap<String, f64>,
    switches: BTreeMap<String, bool>,
}

impl Sample for Step {
    fn get(&self, quantity: &str) -> Option<f64> {
        self.values.get(quantity).copied()
    }
    fn switch_on(&self, switch: &str) -> Option<bool> {
        self.switches.get(switch).copied()
    }
}

fn at(value: f64) -> Step {
    Step {
        values: BTreeMap::from([("v".to_string(), value)]),
        switches: BTreeMap::new(),
    }
}

fn rule(
    measure: PeriodMeasure,
    boundary: PeriodBoundary,
    min: Option<f64>,
    max: Option<f64>,
) -> Rule {
    Rule {
        name: "P".into(),
        target: "v".into(),
        watch: Watch::PerPeriod {
            quantity: "v".into(),
            measure,
            boundary,
            min,
            max,
        },
        unit: String::new(),
        condition: Condition::Above { limit: 0.0 },
        qualifiers: Default::default(),
        severity: Severity::Warning,
        action: Action::Log,
    }
}

/// Drives `samples` (time, value) through an engine and returns everything it reported.
fn drive(rule: Rule, samples: &[(f64, f64)]) -> Vec<EventRecord> {
    let mut engine = RuleEngine::new(vec![rule], false);
    let mut out = Vec::new();
    for (step, (t, value)) in samples.iter().enumerate() {
        out.extend(engine.observe(step, *t, &at(*value)));
    }
    out
}

/// One period of a sine, sampled finely enough to be a fair test of the quadrature.
fn sine_period(n: usize) -> Vec<(f64, f64)> {
    let period = 1e-5;
    (0..=n)
        .map(|i| {
            let t = period * i as f64 / n as f64;
            (t, 10.0 * (2.0 * std::f64::consts::PI * t / period).sin())
        })
        .collect()
}

#[test]
fn the_streaming_rms_agrees_with_the_batch_library() {
    // The property the whole design rests on. A bound that no sine can satisfy forces the
    // measure into the report, where its value can be compared with the library's.
    let samples = sine_period(200);
    let records = drive(
        rule(
            PeriodMeasure::Rms,
            PeriodBoundary::Fixed(1e-5),
            None,
            Some(0.0),
        ),
        &samples,
    );
    assert_eq!(records.len(), 1, "one period closed: {records:?}");
    let streamed = records[0].value;

    let wave: Vec<WaveSample> = samples
        .iter()
        .map(|(t, v)| WaveSample::new(*t, *v))
        .collect();
    let batch = gs_waveform_measurements::rms(&wave, &Window::new(0.0, 1e-5)).expect("batch rms");

    assert!(
        (streamed - batch).abs() <= 1e-12 * batch.abs(),
        "streaming {streamed} vs batch {batch}"
    );
    // And both are the analytic answer for a sine: 10/sqrt(2).
    let analytic = 10.0 / 2.0_f64.sqrt();
    assert!(
        (streamed - analytic).abs() < 2e-3 * analytic,
        "{streamed} vs {analytic}"
    );
}

#[test]
fn the_streaming_mean_and_integral_agree_with_the_batch_library() {
    // A ramp, whose mean and integral are both exactly representable, so a disagreement here is
    // a real one rather than a rounding artefact.
    // Sampled past the boundary on purpose. A fixed period closes on the first step that
    // reaches its end, and `100 * 1e-7` is a hair under `1e-5` in binary floating point, so a
    // run that stops exactly at the boundary may leave the last period open. A real run does
    // not stop there, and a test that did would be testing float representation.
    let samples: Vec<(f64, f64)> = (0..=110).map(|i| (i as f64 * 1e-7, i as f64)).collect();
    let span = 1e-5;

    let mean = drive(
        rule(
            PeriodMeasure::Mean,
            PeriodBoundary::Fixed(span),
            None,
            Some(-1.0),
        ),
        &samples,
    );
    let integral = drive(
        rule(
            PeriodMeasure::Integral,
            PeriodBoundary::Fixed(span),
            None,
            Some(-1.0),
        ),
        &samples,
    );

    let wave: Vec<WaveSample> = samples
        .iter()
        .map(|(t, v)| WaveSample::new(*t, *v))
        .collect();
    let window = Window::new(0.0, span);
    let batch_mean = gs_waveform_measurements::avg(&wave, &window).expect("batch avg");
    let batch_integral = gs_waveform_measurements::integ(&wave, &window).expect("batch integ");

    assert!(
        (mean[0].value - batch_mean).abs() <= 1e-12 * batch_mean.abs(),
        "mean: streaming {} vs batch {batch_mean}",
        mean[0].value
    );
    assert!(
        (integral[0].value - batch_integral).abs() <= 1e-12 * batch_integral.abs(),
        "integral: streaming {} vs batch {batch_integral}",
        integral[0].value
    );
}

#[test]
fn a_balanced_square_wave_integrates_to_zero_and_an_unbalanced_one_does_not() {
    // Volt-second balance, which is the measure this whole check was asked for. A winding that
    // sees +V for half a period and -V for the other half accumulates no flux; one that sees
    // the positive half for longer walks away, and that walk-away is invisible in the waveform
    // until the core saturates.
    let period = 1e-5;
    let balanced: Vec<(f64, f64)> = (0..=100)
        .map(|i| {
            let t = i as f64 * period / 100.0;
            (t, if i < 50 { 10.0 } else { -10.0 })
        })
        .collect();
    let records = drive(
        rule(
            PeriodMeasure::Integral,
            PeriodBoundary::Fixed(period),
            Some(-1e-6),
            Some(1e-6),
        ),
        &balanced,
    );
    assert!(
        records.is_empty(),
        "a balanced winding must not be reported: {:?}",
        records.iter().map(|r| r.value).collect::<Vec<_>>()
    );

    // 60% positive: the integral is 0.6*10*T - 0.4*10*T = 2*T volt-seconds per period.
    let unbalanced: Vec<(f64, f64)> = (0..=100)
        .map(|i| {
            let t = i as f64 * period / 100.0;
            (t, if i < 60 { 10.0 } else { -10.0 })
        })
        .collect();
    let records = drive(
        rule(
            PeriodMeasure::Integral,
            PeriodBoundary::Fixed(period),
            Some(-1e-6),
            Some(1e-6),
        ),
        &unbalanced,
    );
    assert_eq!(records.len(), 1, "{records:?}");
    // 1.9e-5, not the 2e-5 an ideal square wave would give. Between the last +10 sample and
    // the first -10 sample there is one segment that interpolates linearly through zero, and
    // its trapezoid is exactly zero -- so a sampled square wave loses one step's area at each
    // edge. That is a property of the data, not of the accumulator: the library's `integ` over
    // the same samples returns the same number.
    let expected = 1.9 * period;
    assert!(
        (records[0].value - expected).abs() < 0.01 * expected,
        "{} vs {expected}",
        records[0].value
    );
    assert!(records[0]
        .detail
        .as_deref()
        .unwrap_or_default()
        .contains("integral of v"));
}

#[test]
fn the_mean_magnitude_of_a_symmetric_signal_is_not_zero() {
    // The reason `|x|` is integrated by splitting each segment at its zero crossing. Treating
    // the magnitude of a line that crosses zero as a line would report a cancellation that does
    // not happen, and a transformer current -- symmetric about zero -- would come out near
    // nothing when its mean magnitude is exactly what a thermal estimate needs.
    let samples = sine_period(200);
    let records = drive(
        rule(
            PeriodMeasure::AbsMean,
            PeriodBoundary::Fixed(1e-5),
            None,
            Some(0.0),
        ),
        &samples,
    );
    assert_eq!(records.len(), 1);
    // The analytic mean magnitude of a sine is 2A/pi.
    let analytic = 2.0 * 10.0 / std::f64::consts::PI;
    assert!(
        (records[0].value - analytic).abs() < 1e-3 * analytic,
        "{} vs {analytic}",
        records[0].value
    );

    // And its plain mean over the same period is zero, which is the contrast that matters.
    let mean = drive(
        rule(
            PeriodMeasure::Mean,
            PeriodBoundary::Fixed(1e-5),
            Some(1.0),
            None,
        ),
        &samples,
    );
    assert!(mean[0].value.abs() < 1e-9, "{}", mean[0].value);
}

#[test]
fn a_fixed_period_is_measured_over_exactly_one_period() {
    // The boundary is split rather than rounded to the step that noticed it. With a step that
    // does not divide the period, closing late would carry a stray step's worth of integral
    // into the measure -- which for a near-zero volt-second balance is not a small error.
    let period = 1e-5;
    // A constant 1, so the integral over one period must be exactly the period.
    let samples: Vec<(f64, f64)> = (0..=37).map(|i| (i as f64 * 3e-7, 1.0)).collect();
    let records = drive(
        rule(
            PeriodMeasure::Integral,
            PeriodBoundary::Fixed(period),
            None,
            Some(0.0),
        ),
        &samples,
    );
    assert!(!records.is_empty());
    assert!(
        (records[0].value - period).abs() < 1e-18,
        "{} vs {period}",
        records[0].value
    );
}

#[test]
fn a_sync_boundary_takes_each_period_from_one_turn_on_to_the_next() {
    // With a closed loop the period is an outcome, not a setting, so the only honest boundary
    // is the one the circuit produced.
    let mut engine = RuleEngine::new(
        vec![rule(
            PeriodMeasure::Integral,
            PeriodBoundary::Sync("S1".into()),
            None,
            Some(0.0),
        )],
        false,
    );
    let mut out = Vec::new();
    // Turn-ons at steps 1 and 5; a constant 1 between them, so the integral is 4 us.
    for step in 0..7 {
        let on = step == 1 || step == 5;
        let sample = Step {
            values: BTreeMap::from([("v".to_string(), 1.0)]),
            switches: BTreeMap::from([("S1".to_string(), on)]),
        };
        out.extend(engine.observe(step, step as f64 * 1e-6, &sample));
    }
    assert_eq!(out.len(), 1, "one completed period: {out:?}");
    assert!((out[0].value - 4e-6).abs() < 1e-18, "{}", out[0].value);
}

#[test]
fn a_rule_whose_sync_switch_does_not_exist_says_so_rather_than_passing() {
    let mut engine = RuleEngine::new(
        vec![rule(
            PeriodMeasure::Integral,
            PeriodBoundary::Sync("NOPE".into()),
            None,
            Some(0.0),
        )],
        false,
    );
    for step in 0..5 {
        engine.observe(step, step as f64 * 1e-6, &at(1.0));
    }
    let (_, summaries) = engine.finish(5);
    assert_eq!(summaries[0].missing_samples, 5);
    assert!(!summaries[0].passed(), "no boundary means no measurement");
}
