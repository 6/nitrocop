factory :article do
  association :user
end
factory :post do
  association :author, factory: :user
end
factory :comment do
  trait :with_user do
    association :user
  end
end
factory :host do
  trait :managed do
    is_managed { true }
  end
  trait :with_ipv6 do
    managed
  end
  trait :dualstack do
    with_ipv6
  end
  trait :with_associations do
    with_archs
    with_media
    with_ipv6
  end
  trait :with_archs do
    architectures { [create(:architecture)] }
  end
  trait :with_media do
    media { [create(:medium)] }
  end
end
factory :company do
  trait :premium do
    level { 3 }
  end
  factory :special_company do
    premium
  end
end
