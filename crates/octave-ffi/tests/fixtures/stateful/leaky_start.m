% A struct-valued state (not a bare scalar), so save_state/load_state are exercised on the
% shape a real block is likely to use: two fields of different types.
function state = leaky_start()
  state = struct('sum', 0, 'n', int32(0));
end
