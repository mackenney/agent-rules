export function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("timeout")), ms);
    promise.then(
      (v) => { clearTimeout(timer); resolve(v); },
      (e) => { clearTimeout(timer); reject(e); }
    );
  });
}

export function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export function createSemaphore(concurrency: number): {
  acquire(): Promise<void>;
  release(): void;
} {
  let active = 0;
  const queue: Array<() => void> = [];

  return {
    acquire() {
      return new Promise<void>((resolve) => {
        if (active < concurrency) {
          active++;
          resolve();
        } else {
          queue.push(() => {
            active++;
            resolve();
          });
        }
      });
    },
    release() {
      active--;
      const next = queue.shift();
      if (next) next();
    },
  };
}

export interface RetryOpts {
  maxRetries: number;
  baseDelayMs: number;
  shouldRetry: (err: unknown) => boolean;
  onRetry?: (err: unknown, attempt: number) => void;
}

export async function withRetry<T>(fn: () => Promise<T>, opts: RetryOpts): Promise<T> {
  for (let attempt = 0; attempt < opts.maxRetries; attempt++) {
    try {
      return await fn();
    } catch (err) {
      if (!opts.shouldRetry(err)) throw err;
      opts.onRetry?.(err, attempt);
      if (attempt < opts.maxRetries - 1) {
        await delay(opts.baseDelayMs * Math.pow(2, attempt));
      }
    }
  }
  throw new Error("withRetry: exhausted all attempts");
}
