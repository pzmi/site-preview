pub fn run(coordinator: &mut dyn preview_composition::PipelineCoordinator) -> bool {
    coordinator.settle()
}
