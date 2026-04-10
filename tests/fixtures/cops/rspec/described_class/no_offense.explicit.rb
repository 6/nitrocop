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
