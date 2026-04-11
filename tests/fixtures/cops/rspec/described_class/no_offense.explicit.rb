RSpec.describe Alchemy::Configuration do
  let(:configuration) do
    Class.new(described_class) do
      option :auto_logout_time, :integer, default: 30
    end.new
  end
end

RSpec.describe ActiveInteraction::Inputs do
  it "updates execute" do
    described_class.class_exec do
      def execute; end
    end
  end
end

# FP fix: described_class::CONSTANT — OnlyStaticConstants stops recursion
# at const nodes, so described_class inside a constant path is not flagged
RSpec.describe Arachni::Browser::ElementLocator do
  context "and a #{described_class::ARACHNI_ID}" do
    it { described_class::ARACHNI_ID }
  end
end
