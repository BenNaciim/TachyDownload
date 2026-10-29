'use strict';

const { sleep } = require('./util');

/**
 * Shared bandwidth limiter. `rate` is in bytes per second, 0 means unlimited.
 * A caller may take more than the bucket holds: the balance goes negative and
 * the caller waits until it has been repaid, which keeps the long-run rate exact.
 */
class TokenBucket {
  constructor(rate = 0) {
    this.setRate(rate);
  }

  setRate(rate) {
    this.rate = Math.max(0, Number(rate) || 0);
    this.tokens = this.rate;
    this.last = Date.now();
  }

  _refill() {
    const now = Date.now();
    if (this.rate > 0) {
      this.tokens = Math.min(this.rate, this.tokens + ((now - this.last) / 1000) * this.rate);
    }
    this.last = now;
  }

  async take(bytes, signal) {
    if (this.rate <= 0) return;
    this._refill();
    this.tokens -= bytes;
    if (this.tokens < 0) {
      await sleep((-this.tokens / this.rate) * 1000, signal);
    }
  }
}

module.exports = { TokenBucket };
