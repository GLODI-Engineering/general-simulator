function [new_state, y] = accumulator(state, t, dt, u)
  new_state = state + u;
  y = new_state;
end
