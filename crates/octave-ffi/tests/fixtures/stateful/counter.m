% Deliberately does NOT advance `state` itself -- the "output/update split" contract expects
% `_update.m`, when present, to be the place discrete bookkeeping commits forward, not `output`.
function [new_state, y] = counter(state, t, dt, u)
  new_state = state;
  y = state + u;
end
