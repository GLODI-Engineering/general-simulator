function [new_state, y] = xc_charge_output_xc(state, t, dt, u, xc)
  new_state = state;
  y = xc(1);
end
