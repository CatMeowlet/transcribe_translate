import { useState } from 'react';
import { Input } from './components/ui/input';
import { Button } from './components/ui/button';

function App() {
	const [room, setRoom] = useState('');
	const [ws, setWs] = useState(null);

	const connectToRoom = () => {
		if (!room.trim()) {
			alert('Please enter a room name.');
			return;
		}

		const socket = new WebSocket(`ws://127.0.0.1:12345/${room}`);
		socket.onopen = () => {
			console.log(`Connected to room: ${room}`);
		};
		socket.onmessage = (event) => {
			console.log('Message from server:', event.data);
		};
		socket.onclose = () => {
			console.log(`Disconnected from room: ${room}`);
		};
		socket.onerror = (err) => {
			console.error('WebSocket error:', err);
		};

		setWs(socket);
	};

	return (
		<div className='max-w-2xl mx-auto py-10'>
			<div className='flex w-full justify-center gap-2'>
				<Input
					value={room}
					onChange={(e) => setRoom(e.target.value)}
					placeholder='Enter room name...'
					className='w-64'
				/>
				<Button onClick={connectToRoom}>Connect</Button>
			</div>
		</div>
	);
}

export default App;
