factory :article do
  user
  ^^ FactoryBot/AssociationStyle: Use explicit style to define associations.
end
factory :post do
  author factory: %i[user admin]
  ^^^^^^ FactoryBot/AssociationStyle: Use explicit style to define associations.
end
factory :comment do
  trait :with_user do
    reviewer
    ^^^^^^^^ FactoryBot/AssociationStyle: Use explicit style to define associations.
  end
end
factory :account do
  password_confirmation(&:password)
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ FactoryBot/AssociationStyle: Use explicit style to define associations.
end
factory :review do
  %w[a b].each do |n|
    trait :"#{n}_type" do
      reviewer
      ^^^^^^^^ FactoryBot/AssociationStyle: Use explicit style to define associations.
    end
  end
end
