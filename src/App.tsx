import { useEffect, useState } from 'react';
import './App.css';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { LogIn } from './views/LogIn';
import { List } from './views/List';
import { Menu } from './components/Menu';

type Broadcaster = {
  broadcaster_id: string;
  broadcaster_name: string;
  category: string;
  title: string;
  is_live: boolean;
};

type Broadcasters = {
  online: Broadcaster[];
  offline: Broadcaster[];
};

export const App = () => {
  const [layout, setLayout] = useState<string>('list');
  const [loading, setLoading] = useState<boolean>(true);
  useEffect(() => {
    invoke('on_startup').then((val) => {
      if (val === 'log_in') {
        setLayout('login');
      } else {
        invoke('fetch_streamers');
      }
    });
    let unlistenLoggedIn: UnlistenFn;
    let unlistenStreamers: UnlistenFn;

    listen('logged_in', (_event) => {
      setLayout('list');
    }).then((fn) => {
      unlistenLoggedIn = fn;
    });

    listen('streamers:fetched', (event) => {
      const { offline, online } = event.payload as Broadcasters;
      setLoading(false);
    }).then((fn) => {
      unlistenStreamers = fn;
    });
    return () => {
      unlistenLoggedIn && unlistenLoggedIn();
      unlistenStreamers && unlistenStreamers();
    };
  }, []);

  return (
    <div className="w-full h-full dark:bg-[#26262c] bg-[#efeff1] flex flex-col">
      <Menu />
      {layout === 'login' ? <LogIn /> : layout === 'list' && <List loading={loading} />}
    </div>
  );
};
