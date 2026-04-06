expect { run }.to change(User, :count).by(1)
expect { run }.to change(Post, :count)
expect { run }.to change { User.sum(:points) }
expect { run }.to change { user.reload.name }
expect(run).to change(Order, :total)
Record.change { User.count }
# Block form with local variable receiver is not flagged (RuboCop only flags bare method calls/constants)
cron_entry = find(:example)
my_local = find(:other)
expect { run }.to change { cron_entry.enabled? }
expect { run }.to change { my_local.name }
expect { run }.to change { Sidekiq.redis { |conn| conn.zcard("schedule") } }
# Global variable receiver in block form — not flagged (not a constant or bare method call)
expect { run }.to change { $token.value }
# Instance variable receiver in block form — not flagged
expect { run }.to change { @user.name }
# Class variable receiver in block form — not flagged
expect { run }.to change { @@count.value }
# Chained method call receiver in block form — not flagged (receiver has its own receiver)
expect { run }.to change { users.active.count }
