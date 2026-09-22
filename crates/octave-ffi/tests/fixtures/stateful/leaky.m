% A leaky accumulator: sum <- 0.9*sum + u, n <- n + 1. The 0.9 factor makes every step's value
% carry a full-precision fraction, so a checkpoint round trip that lost a bit would show.
function [new_state, y] = leaky(state, t, dt, u)
  new_state = state;
  new_state.sum = 0.9 * state.sum + u;
  new_state.n = state.n + 1;
  y = new_state.sum;
end
