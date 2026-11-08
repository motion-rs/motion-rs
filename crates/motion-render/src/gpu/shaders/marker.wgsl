// Empty dispatch that gives a timestamp marker pass real work: Metal records no sample for an
// encoder that executes nothing, so a pass without a dispatch would read back as zero.
@compute @workgroup_size(1)
fn marker() {}
