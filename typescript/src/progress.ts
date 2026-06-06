import chalk from "chalk";

export interface ProgressReporter {
  /** Call when any LLM call is about to fire (updates displayed label). */
  onCallStart(label: string): void;
  /** Call when any LLM call returns — increments done, advances the bar. */
  onCallDone(label: string): void;
  /** Dynamically grow the total when an agentic escalation is discovered. */
  addTotal(n: number): void;
  /** Write a diagnostic line without corrupting the spinner. */
  log(msg: string): void;
  /** Flush final line and stop the spinner. */
  stop(): void;
}

const SPINNER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"] as const;

function progressBar(done: number, total: number, width = 22): string {
  const ratio = total > 0 ? done / total : 0;
  const filled = Math.round(ratio * width);
  return `[${'█'.repeat(filled)}${'░'.repeat(width - filled)}]`;
}

export function createProgressReporter(total: number): ProgressReporter {
  const isTTY = process.stderr.isTTY === true;
  const isCI = !!process.env["CI"] || !!process.env["GITHUB_ACTIONS"];

  if (total === 0 || (!isTTY && !isCI)) {
    return { onCallStart: () => {}, onCallDone: () => {}, addTotal: () => {}, log: (msg) => process.stderr.write(msg + "\n"), stop: () => {} };
  }

  let currentTotal = total;
  let done = 0;

  if (isCI) {
    return {
      onCallStart: () => {},
      onCallDone: (label) => {
        done++;
        process.stderr.write(`[${done}/${currentTotal}] ${label}\n`);
      },
      addTotal: (n) => { currentTotal += n; },
      log: (msg) => process.stderr.write(msg + "\n"),
      stop: () => {},
    };
  }

  // Animated spinner for interactive TTY.
  let frameIdx = 0;
  let activeLabel = "";

  function render(): void {
    const frame = chalk.cyan(SPINNER_FRAMES[frameIdx % SPINNER_FRAMES.length]!);
    const bar = progressBar(done, currentTotal);
    const short = activeLabel.length > 72 ? `…${activeLabel.slice(-71)}` : activeLabel;
    process.stderr.write(
      `\r\x1b[K${frame} ${chalk.dim(bar)} ${chalk.bold(`${done}/${currentTotal}`)}  ${chalk.cyan(short)}`
    );
    frameIdx++;
  }

  const timer = setInterval(render, 80);
  (timer as NodeJS.Timeout & { unref?(): void }).unref?.();

  return {
    onCallStart(label) {
      activeLabel = label;
    },
    onCallDone(label) {
      done++;
      activeLabel = label;
    },
    addTotal(n) {
      currentTotal += n;
    },
    log(msg) {
      process.stderr.write(`\r\x1b[K${msg}\n`);
    },
    stop() {
      clearInterval(timer);
      const bar = progressBar(done, currentTotal);
      process.stderr.write(
        `\r\x1b[K${chalk.dim(bar)} ${chalk.bold(`${done}/${currentTotal}`)}${chalk.green(" ✓")}\n`
      );
    },
  };
}
