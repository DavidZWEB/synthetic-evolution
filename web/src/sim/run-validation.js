/**
 * Serializes asynchronous run-config validation.
 *
 * Only the newest request may replace the active world. Cancelling also advances the
 * token, so a delayed response cannot revive a run after a later malformed URL.
 */

export function createRunValidation({ getSim, onPendingChange, onAccepted, onRejected }) {
  let requestId = 0;
  let pending = null;

  function cancel() {
    requestId += 1;
    pending = null;
    onPendingChange(false);
  }

  return {
    request(run) {
      requestId += 1;
      pending = { ...run, requestId };
      onPendingChange(true);
      getSim()?.validateRun(run.seed, run.founders, run.params, requestId);
    },

    accept(message) {
      if (!pending || message.requestId !== requestId || message.requestId !== pending.requestId) {
        return false;
      }
      const requested = pending;
      pending = null;
      onPendingChange(false);
      if (message.error) onRejected(message.error);
      else onAccepted({ ...requested, params: message.params });
      return true;
    },

    cancel,
  };
}
