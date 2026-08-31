function [new_state, y] = accumulate(state, t, dt, u)
  new_state = state + u;
  y = new_state;
end
