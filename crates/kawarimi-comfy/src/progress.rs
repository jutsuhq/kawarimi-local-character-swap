//! How far a render's sampling is, from the progress bars ComfyUI writes to its log: each window's sampler draws
//! one bar of `steps`, redrawn in place with `\r`.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sampling {
    /// Windows whose sampling finished.
    pub done: u32,
    /// Steps of the window being sampled.
    pub step: u32,
}

/// The bars in `log` (the part written since the render started) that are `steps` long.
pub fn sampling(log: &str, steps: u32) -> Sampling {
    let mut out = Sampling::default();
    let mut last: Option<u32> = None;
    for line in log.split(['\r', '\n']) {
        let Some((value, total)) = bar(line) else { continue };
        if total != steps {
            continue;
        }
        if last.is_some_and(|l| value < l) {
            out.step = 0;
        }
        if value == total {
            if last != Some(total) {
                out.done += 1;
            }
            out.step = 0;
        } else {
            out.step = value;
        }
        last = Some(value);
    }
    out
}

/// "` 50%|#####     | 2/4 [00:40<00:40, 20.44s/it]`" → (2, 4).
fn bar(line: &str) -> Option<(u32, u32)> {
    let (_, rest) = line.split_once("| ")?;
    let (count, _) = rest.split_once(" [")?;
    let (value, total) = count.trim().split_once('/')?;
    Some((value.parse().ok()?, total.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: &str = "got prompt\n  0%|          | 0/4 [00:00<?, ?it/s]\r 25%|##5       | 1/4 [00:37<01:52, 37.45s/it]\r\
        \r 50%|#####     | 2/4 [00:40<00:40, 20.44s/it]\r100%|##########| 4/4 [01:22<00:00, 20.63s/it]\
        \r100%|##########| 4/4 [01:22<00:00, 20.52s/it]\n";

    #[test]
    fn counts_windows_and_steps() {
        assert_eq!(sampling("", 4), Sampling { done: 0, step: 0 });
        let pose = "DWPose: 100%|##########| 33/33 [00:06<00:00]\n";
        assert_eq!(sampling(&format!("{pose}{WINDOW}"), 4), Sampling { done: 1, step: 0 });
        let second = " 0%|          | 0/4 [00:00<?, ?it/s]\r 75%|#######5  | 3/4 [01:01<00:20, 20.55s/it]";
        assert_eq!(sampling(&format!("{WINDOW}{second}"), 4), Sampling { done: 1, step: 3 });
        assert_eq!(sampling(&format!("{WINDOW}{WINDOW}"), 4), Sampling { done: 2, step: 0 });
    }
}
