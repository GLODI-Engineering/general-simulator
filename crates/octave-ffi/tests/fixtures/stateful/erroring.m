function [new_state, y] = erroring(state, t, dt, u)
  if u < 0
    error('negative input not allowed');
  end
  new_state = state + u;
  y = new_state;
end
