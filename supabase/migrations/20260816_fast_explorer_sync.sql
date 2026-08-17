create table if not exists public.fast_explorer_devices (
    id uuid primary key default gen_random_uuid(),
    user_id uuid not null references auth.users(id) on delete cascade,
    device_key text not null,
    name text not null check (char_length(name) between 1 and 120),
    platform text not null default 'unknown',
    last_seen_at timestamptz not null default now(),
    created_at timestamptz not null default now(),
    unique (user_id, device_key)
);

create table if not exists public.fast_explorer_transfers (
    id uuid primary key,
    user_id uuid not null references auth.users(id) on delete cascade,
    sender_device_id uuid not null references public.fast_explorer_devices(id) on delete cascade,
    receiver_device_id uuid not null references public.fast_explorer_devices(id) on delete cascade,
    kind text not null check (kind in ('file', 'clipboard')),
    file_name text,
    object_path text,
    clipboard_text text,
    status text not null default 'pending' check (status in ('pending', 'received')),
    created_at timestamptz not null default now(),
    received_at timestamptz,
    check ((kind = 'file' and object_path is not null and file_name is not null and clipboard_text is null)
        or (kind = 'clipboard' and clipboard_text is not null and object_path is null))
);

create index if not exists fast_explorer_transfers_receiver_pending_idx
    on public.fast_explorer_transfers(receiver_device_id, status, created_at);
create index if not exists fast_explorer_transfers_user_idx
    on public.fast_explorer_transfers(user_id);

alter table public.fast_explorer_devices enable row level security;
alter table public.fast_explorer_transfers enable row level security;

revoke all on public.fast_explorer_devices from anon;
revoke all on public.fast_explorer_transfers from anon;
grant select, insert, update on public.fast_explorer_devices to authenticated;
grant select, insert, delete on public.fast_explorer_transfers to authenticated;

drop policy if exists "fast explorer devices own rows" on public.fast_explorer_devices;
create policy "fast explorer devices own rows"
on public.fast_explorer_devices
for all
to authenticated
using (user_id = (select auth.uid()))
with check (user_id = (select auth.uid()));

drop policy if exists "fast explorer transfers read own" on public.fast_explorer_transfers;
create policy "fast explorer transfers read own"
on public.fast_explorer_transfers
for select
to authenticated
using (user_id = (select auth.uid()));

drop policy if exists "fast explorer transfers insert own devices" on public.fast_explorer_transfers;
create policy "fast explorer transfers insert own devices"
on public.fast_explorer_transfers
for insert
to authenticated
with check (
    user_id = (select auth.uid())
    and exists (
        select 1 from public.fast_explorer_devices sender
        where sender.id = sender_device_id and sender.user_id = (select auth.uid())
    )
    and exists (
        select 1 from public.fast_explorer_devices receiver
        where receiver.id = receiver_device_id and receiver.user_id = (select auth.uid())
    )
);

drop policy if exists "fast explorer transfers update own received" on public.fast_explorer_transfers;

drop policy if exists "fast explorer transfers delete own" on public.fast_explorer_transfers;
create policy "fast explorer transfers delete own"
on public.fast_explorer_transfers
for delete
to authenticated
using (user_id = (select auth.uid()));

insert into storage.buckets (id, name, public)
values ('fast-explorer-transfers', 'fast-explorer-transfers', false)
on conflict (id) do update set public = false;

drop policy if exists "fast explorer storage upload own folder" on storage.objects;
create policy "fast explorer storage upload own folder"
on storage.objects
for insert
to authenticated
with check (
    bucket_id = 'fast-explorer-transfers'
    and (storage.foldername(name))[1] = (select auth.uid())::text
);

drop policy if exists "fast explorer storage read own folder" on storage.objects;
create policy "fast explorer storage read own folder"
on storage.objects
for select
to authenticated
using (
    bucket_id = 'fast-explorer-transfers'
    and (storage.foldername(name))[1] = (select auth.uid())::text
);

drop policy if exists "fast explorer storage delete own folder" on storage.objects;
create policy "fast explorer storage delete own folder"
on storage.objects
for delete
to authenticated
using (
    bucket_id = 'fast-explorer-transfers'
    and (storage.foldername(name))[1] = (select auth.uid())::text
);

comment on table public.fast_explorer_devices is
    'FastExplorer devices registered under a Supabase Auth user.';
comment on table public.fast_explorer_transfers is
    'FastExplorer pending file and clipboard transfers; file payloads live in the private Storage bucket.';
