% Pure -- must not (and does not) reassign `state`. xc_dot = 1 - xc (a first-order charge),
% ignoring the unused input u (present only to exercise argument passing).
function xc_dot = xc_charge_derivative(state, u, xc)
  xc_dot = 1 - xc;
end
