function [new_state, y] = ticker(state, t, dt, u)
  new_state = state + 1;
  y = new_state;
end
