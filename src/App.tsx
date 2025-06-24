import { useEffect } from 'react';
import './App.css';
import { Glitch } from './components/Glitch';
import { Button } from './components/ui/button';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export const App = () => {
  useEffect(() => {
    listen("logged_in", (event) => {
      invoke('shutdown_server');
    })
  }, []);

  return (
    <div className="flex flex-col h-screen justify-center align-center">
      <div className="flex justify-center">
        <Button
          onClick={() => {
            invoke('login');
          }}
          className={`bg-[#9146FF] text-center align-center cursor-pointer`}
        >
          <Glitch />
          Login with Twitch
        </Button>
      </div>
    </div>
  );
};
