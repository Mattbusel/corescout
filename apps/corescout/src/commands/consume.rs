//! The consuming commands: `inspect`, `replay`, `observe`, `learn`, `model`,
//! `latent`, `describe`.
//!
//! # What these commands are not allowed to do
//!
//! None of them reads hardware. Everything they know arrives through a
//! `Source`, which is either a live plane or a recording and which they cannot
//! tell apart. `corescout learn` and `corescout learn --source trace.jsonl` run
//! the same lines of code over the same types.
//!
//! That is the claim the whole project rests on, so it is worth being precise
//! about what it means here: these functions have no import from
//! `corescout_substrate`, and the standalone binaries that expose the same
//! functionality do not link that crate at all. In this module the separation is
//! a convention; in `mirror-observer` and `mirror-learn` it is enforced by the
//! linker.
//!
//! # The two-ontology experiment
//!
//! `observe --both` is the experiment the project was reorganised around: run
//! one observer that is given our names for things and one that is given only
//! identity, state, relations and time, then compare what each discovered. The
//! comparison is printed whichever way it comes out.

use corescout_core::error::{Error, Result};
use corescout_human::Format;
use corescout_identity::{describe as build_description, Evidence};
use corescout_mirror::MirrorSnapshot;
use corescout_observer::lens::Lens;
use corescout_observer::observer::{Observer, ObserverConfig};
use corescout_represent::latent::LatentCatalogue;
use corescout_represent::normalize::Normalizer;
use corescout_selfmodel::SelfModel;

use crate::cli::{Ontology, SourceOptions};
use crate::runtime;

/// Read the source and describe what is in it.
pub fn inspect(options: &SourceOptions, format: Format, filter: Option<&str>) -> Result<()> {
    let mut source = runtime::open_source(options)?;
    let frames = source.take(options.frames.unwrap_or(1))?;
    let latest = frames.last().ok_or_else(empty_source)?;
    match format {
        Format::Json => println!("{}", corescout_human::json::snapshot(latest)?),
        Format::Human => {
            print!("{}", corescout_human::mirror_debug::summary(latest));
            print!("{}", corescout_human::mirror_debug::detail(latest, filter));
        }
    }
    Ok(())
}

/// Read a recording as though it were live.
pub fn replay(options: &SourceOptions, format: Format) -> Result<()> {
    let mut source = runtime::open_source(options)?;
    let frames = runtime::take_frames(&mut source, options.frames)?;
    if frames.is_empty() {
        return Err(empty_source());
    }
    match format {
        Format::Json => {
            for frame in &frames {
                println!("{}", corescout_human::json::snapshot(frame)?);
            }
        }
        Format::Human => {
            let first = &frames[0];
            let last = &frames[frames.len() - 1];
            println!(
                "{} reflections, sequence {} to {}, spanning {:.1} s",
                frames.len(),
                first.sequence,
                last.sequence,
                (last.monotonic_ns.saturating_sub(first.monotonic_ns)) as f64 / 1e9
            );
            print!("{}", corescout_human::mirror_debug::summary(last));
        }
    }
    Ok(())
}

/// Find structure in the reflection.
pub fn observe(options: &SourceOptions, ontology: Ontology, format: Format) -> Result<()> {
    let mut source = runtime::open_source(options)?;
    let frames = runtime::take_frames(&mut source, options.frames.or(Some(600)))?;
    if frames.len() < 8 {
        return Err(Error::invalid(format!(
            "structure needs more than {} reflections; record a longer trace",
            frames.len()
        )));
    }

    let config = ObserverConfig::default();
    match ontology {
        Ontology::Both => {
            let labelled = run_observer(Lens::Labelled, config.clone(), &frames);
            let unlabelled = run_observer(Lens::Unlabelled, config, &frames);
            match (labelled, unlabelled) {
                (Some(a), Some(b)) => {
                    if format == Format::Json {
                        // Findings render as prose rather than a struct: the
                        // comparison is the result, and flattening it into
                        // fields would lose which discovery belongs to which
                        // observer.
                        println!(
                            "{}",
                            runtime::to_json(&serde_json::json!({
                                "labelled": a.render(),
                                "unlabelled": b.render(),
                                "comparison": a.compare(&b),
                                "same_structure": a.discovered_the_same_structure(&b),
                            }))?
                        );
                    } else {
                        // The comparison is printed whichever way it came out.
                        // A result that embarrasses our ontology is the result
                        // the experiment was run to find.
                        print!("{}", a.compare(&b));
                    }
                }
                _ => return Err(nothing_discovered()),
            }
        }
        single => {
            let lens = if single == Ontology::Labelled {
                Lens::Labelled
            } else {
                Lens::Unlabelled
            };
            let findings = run_observer(lens, config, &frames).ok_or_else(nothing_discovered)?;
            match format {
                Format::Json => println!("{}", runtime::to_json(&findings.render())?),
                Format::Human => print!("{}", findings.render()),
            }
        }
    }
    Ok(())
}

/// Learn to predict the next reflection, and score the predictions.
pub fn learn(options: &SourceOptions, format: Format) -> Result<()> {
    let mut source = runtime::open_source(options)?;
    let frames = runtime::take_frames(&mut source, options.frames.or(Some(2000)))?;
    let learned = learn_from(&frames)?;
    match format {
        Format::Json => println!("{}", runtime::to_json(&learned.model)?),
        Format::Human => print!("{}", render_learning(&learned)),
    }
    Ok(())
}

/// Report what the self-model currently believes.
pub fn model(options: &SourceOptions, format: Format) -> Result<()> {
    let mut source = runtime::open_source(options)?;
    let frames = runtime::take_frames(&mut source, options.frames.or(Some(2000)))?;
    let learned = learn_from(&frames)?;
    match format {
        Format::Json => println!("{}", runtime::to_json(&learned.model)?),
        Format::Human => {
            println!(
                "the model has scored {} predictions across {} reflections",
                learned.scored,
                frames.len()
            );
            println!(
                "mean skill against assuming no change: {:+.3}",
                learned.skill
            );
            println!(
                "{}",
                if learned.skill > 0.0 {
                    "positive skill means the model beats the assumption that nothing changes"
                } else {
                    // Reported in the same voice as the good case.
                    "the model does not beat the assumption that nothing changes"
                }
            );
        }
    }
    Ok(())
}

/// List the states the machine discovered in itself.
pub fn latent(options: &SourceOptions, format: Format) -> Result<()> {
    let mut source = runtime::open_source(options)?;
    let frames = runtime::take_frames(&mut source, options.frames.or(Some(2000)))?;
    let learned = learn_from(&frames)?;
    match format {
        Format::Json => println!("{}", runtime::to_json(&learned.catalogue)?),
        Format::Human => {
            if learned.catalogue.is_empty() {
                println!("no recurring states were distinguishable in these reflections");
                return Ok(());
            }
            println!(
                "{} states discovered over {} observations",
                learned.catalogue.len(),
                learned.catalogue.observations()
            );
            for state in learned.catalogue.ranked() {
                println!(
                    "  {}  entered {} times, mean dwell {:.0} ms",
                    state.id,
                    state.entries,
                    state.mean_dwell_ns() / 1e6
                );
            }
        }
    }
    Ok(())
}

/// Say what the machine can truthfully say about itself.
pub fn describe(options: &SourceOptions, format: Format) -> Result<()> {
    let mut source = runtime::open_source(options)?;
    let frames = runtime::take_frames(&mut source, options.frames.or(Some(2000)))?;
    let latest = frames.last().ok_or_else(empty_source)?.clone();
    let learned = learn_from(&frames)?;

    let evidence = Evidence {
        frames: frames.len(),
        scored_predictions: learned.scored,
        model_skill: (learned.scored > 0).then_some(learned.skill),
        // Nothing here acted, so there is no influence evidence and no
        // proposition about influence will be produced.
        actions_taken: 0,
        responsive_entities: 0,
        largest_variable_group: None,
    };
    let description = build_description(Some(&latest), Some(&learned.catalogue), &[], &evidence);
    match format {
        Format::Json => println!("{}", runtime::to_json(&description)?),
        Format::Human => print!("{}", description.render()),
    }
    Ok(())
}

/// What one learning pass produced.
pub struct Learned {
    pub model: SelfModel,
    pub catalogue: LatentCatalogue,
    pub scored: u64,
    pub skill: f64,
}

/// Fit a normaliser, recognise states, and learn to predict, in one pass.
///
/// Exposed because `demo self` and `mirror-learn` need exactly this and the
/// alternative is three copies that drift.
pub fn learn_from(frames: &[MirrorSnapshot]) -> Result<Learned> {
    if frames.len() < 8 {
        return Err(Error::invalid(format!(
            "learning needs more than {} reflections; record a longer trace",
            frames.len()
        )));
    }

    // Fit the scaling on an early slice rather than the whole trace: fitting on
    // data the model is then scored against is how a model comes to look better
    // than it is.
    let fit_frames = (frames.len() / 5).clamp(4, 200);
    let rows: Vec<Vec<f64>> = frames[..fit_frames]
        .iter()
        .map(|frame| frame.state.as_slice().to_vec())
        .collect();
    let cols = frames[0].channels.len().max(1);
    let normalizer = Normalizer::fit(&rows, cols);

    let mut catalogue = LatentCatalogue::new(0.75, 32);
    let mut model = SelfModel::default();
    let mut scored = 0u64;
    let mut skill_total = 0.0;

    for frame in frames {
        let (point, _filled) = normalizer.apply_filled(frame.state.as_slice(), cols);
        catalogue.observe(&point, frame.monotonic_ns);
        let before = model.scored();
        model.observe(frame);
        if model.scored() > before {
            scored += 1;
            skill_total += model.skill();
        }
    }

    let skill = if scored > 0 {
        skill_total / scored as f64
    } else {
        0.0
    };
    Ok(Learned {
        model,
        catalogue,
        scored,
        skill,
    })
}

fn run_observer(
    lens: Lens,
    config: ObserverConfig,
    frames: &[MirrorSnapshot],
) -> Option<corescout_observer::report::Findings> {
    let mut observer = Observer::new(lens, config);
    for frame in frames {
        observer.observe(frame);
    }
    observer.findings()
}

fn render_learning(learned: &Learned) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} states discovered, {} predictions scored\n",
        learned.catalogue.len(),
        learned.scored
    ));
    out.push_str(&format!(
        "mean skill against assuming no change: {:+.3}\n",
        learned.skill
    ));
    if learned.scored == 0 {
        out.push_str("nothing was scored, so nothing is claimed about the model\n");
    }
    out
}

fn empty_source() -> Error {
    Error::invalid(
        "the source produced no reflections; is the mirror running, or is the trace empty?",
    )
}

fn nothing_discovered() -> Error {
    Error::invalid("no structure was found in these reflections")
}
